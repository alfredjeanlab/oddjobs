// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Daemon lifecycle management: startup, shutdown, recovery.

mod reconcile;
pub(crate) use reconcile::reconcile_state;

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Instant;

use fs2::FileExt;
use oj_adapters::{
    ClaudeAgentAdapter, DesktopNotifyAdapter, TmuxAdapter, TracedAgent, TracedSession,
};
use oj_core::{Event, SystemClock};
use oj_engine::breadcrumb::{self, Breadcrumb};
use oj_engine::{
    AgentLogger, MetricsHealth, Runtime, RuntimeConfig, RuntimeDeps, UsageMetricsCollector,
};
use oj_storage::{load_snapshot, Checkpointer, MaterializedState, Wal};
use thiserror::Error;
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::event_bus::{EventBus, EventReader};

/// Daemon runtime with concrete adapter types (wrapped with tracing)
pub type DaemonRuntime = Runtime<
    TracedSession<TmuxAdapter>,
    TracedAgent<ClaudeAgentAdapter<TracedSession<TmuxAdapter>>>,
    DesktopNotifyAdapter,
    SystemClock,
>;

/// Daemon configuration
#[derive(Debug, Clone)]
pub struct Config {
    /// Root state directory (e.g. ~/.local/state/oj)
    pub state_dir: PathBuf,
    /// Path to Unix socket
    pub socket_path: PathBuf,
    /// Path to lock/PID file
    pub lock_path: PathBuf,
    /// Path to version file
    pub version_path: PathBuf,
    /// Path to daemon log file
    pub log_path: PathBuf,
    /// Path to WAL file
    pub wal_path: PathBuf,
    /// Path to snapshot file
    pub snapshot_path: PathBuf,
    /// Path to workspaces directory
    pub workspaces_path: PathBuf,
    /// Path to per-job log files
    pub logs_path: PathBuf,
}

impl Config {
    /// Load configuration for the user-level daemon.
    ///
    /// Uses fixed paths under `~/.local/state/oj/` (or `$XDG_STATE_HOME/oj/`).
    /// One daemon serves all projects for a user.
    pub fn load() -> Result<Self, LifecycleError> {
        let state_dir = state_dir()?;

        Ok(Self {
            socket_path: state_dir.join("daemon.sock"),
            lock_path: state_dir.join("daemon.pid"),
            version_path: state_dir.join("daemon.version"),
            log_path: state_dir.join("daemon.log"),
            wal_path: state_dir.join("wal").join("events.wal"),
            snapshot_path: state_dir.join("snapshot.json"),
            workspaces_path: state_dir.join("workspaces"),
            logs_path: state_dir.join("logs"),
            state_dir,
        })
    }
}

/// Daemon state during operation.
///
/// The listener is returned separately from startup to be spawned as a Listener task.
pub struct DaemonState {
    /// Configuration
    pub config: Config,
    // NOTE(lifetime): Held to maintain exclusive file lock; released on drop
    #[allow(dead_code)]
    lock_file: File,
    /// Materialized state (shared with runtime and listener)
    pub state: Arc<Mutex<MaterializedState>>,
    /// Runtime for event processing (Arc for sharing with background reconciliation)
    pub runtime: Arc<DaemonRuntime>,
    /// Event bus for crash recovery
    pub event_bus: EventBus,
    /// When daemon started
    pub start_time: Instant,
    /// Orphaned jobs detected from breadcrumbs at startup
    pub orphans: Arc<Mutex<Vec<Breadcrumb>>>,
    /// Metrics collector health handle
    pub metrics_health: Arc<Mutex<MetricsHealth>>,
}

/// Result of daemon startup - includes both the daemon state and the listener.
pub struct StartupResult {
    /// The daemon state for event processing
    pub daemon: DaemonState,
    /// The Unix socket listener to spawn as a task
    pub listener: UnixListener,
    /// Event reader for the engine loop
    pub event_reader: EventReader,
    /// Context for running reconciliation as a background task
    pub reconcile_ctx: ReconcileCtx,
}

/// Data needed to run reconciliation as a background task.
///
/// Reconciliation is deferred until after READY is printed so the daemon
/// is immediately responsive to CLI commands.
pub struct ReconcileCtx {
    /// Runtime for agent recovery operations
    pub runtime: Arc<DaemonRuntime>,
    /// Snapshot of state at startup (avoids holding mutex during reconciliation)
    pub state_snapshot: MaterializedState,
    /// Session adapter for checking tmux liveness
    pub session_adapter: TracedSession<TmuxAdapter>,
    /// Channel for emitting events discovered during reconciliation
    pub event_tx: mpsc::Sender<Event>,
    /// Number of non-terminal jobs to reconcile
    pub job_count: usize,
    /// Number of workers with status "running" to reconcile
    pub worker_count: usize,
    /// Number of crons with status "running" to reconcile
    pub cron_count: usize,
    /// Number of non-terminal standalone agent runs to reconcile
    pub agent_run_count: usize,
}

impl DaemonState {
    /// Process an event through the runtime.
    ///
    /// Result events are persisted to the WAL and will be processed by the
    /// engine loop on the next iteration. We deliberately do NOT process them
    /// locally to avoid double-delivery: the engine loop already reads every
    /// WAL entry exactly once, so processing here as well would cause handlers
    /// to fire twice (e.g. duplicate job creation from WorkerPollComplete).
    pub async fn process_event(&mut self, event: Event) -> Result<(), LifecycleError> {
        // Apply the incoming event to materialized state so queries see it.
        // (Effect::Emit events are also applied in the executor for immediate
        // visibility; apply_event is idempotent so the second apply when those
        // events return from the WAL is harmless.)
        {
            let mut state = self.state.lock();
            state.apply_event(&event);
        }

        let result_events = self
            .runtime
            .handle_event(event)
            .await
            .map_err(|e| LifecycleError::Runtime(e.to_string()))?;

        // Persist result events to WAL — the engine loop will read and process
        // them on the next iteration, ensuring single delivery.
        for result_event in result_events {
            if let Err(e) = self.event_bus.send(result_event) {
                warn!("Failed to persist runtime result event to WAL: {}", e);
            }
        }

        Ok(())
    }

    /// Shutdown the daemon gracefully.
    ///
    /// Sessions (tmux) are intentionally preserved across daemon restarts so that
    /// long-running agents continue processing. On next startup, `reconcile_state`
    /// reconnects to surviving sessions. Use `Request::Shutdown { kill: true }` to
    /// terminate all sessions before stopping (handled in the listener before
    /// the shutdown signal is sent, so that kills complete before the CLI starts
    /// its exit timer).
    pub fn shutdown(&mut self) -> Result<(), LifecycleError> {
        info!("Shutting down daemon...");

        // 0. Flush buffered WAL events to disk before tearing down
        if let Err(e) = self.event_bus.flush() {
            warn!("Failed to flush WAL on shutdown: {}", e);
        }

        // 0b. Save final snapshot so next startup doesn't need to replay WAL
        // Uses synchronous checkpoint with compression for fast subsequent startup
        let processed_seq = self.event_bus.processed_seq();
        if processed_seq > 0 {
            let state_clone = self.state.lock().clone();
            let checkpointer = Checkpointer::new(self.config.snapshot_path.clone());
            match checkpointer.checkpoint_sync(processed_seq, &state_clone) {
                Ok(result) => info!(
                    seq = result.seq,
                    size_bytes = result.size_bytes,
                    "saved final shutdown snapshot"
                ),
                Err(e) => warn!("Failed to save shutdown snapshot: {}", e),
            }
        }

        // 1. Remove socket file (listener task stops when tokio runtime exits)
        if self.config.socket_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.config.socket_path) {
                warn!("Failed to remove socket file: {}", e);
            }
        }

        // 2. Remove PID file
        if self.config.lock_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.config.lock_path) {
                warn!("Failed to remove PID file: {}", e);
            }
        }

        // 3. Remove version file
        if self.config.version_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.config.version_path) {
                warn!("Failed to remove version file: {}", e);
            }
        }

        // 4. Lock file is released automatically when self.lock_file is dropped

        info!("Daemon shutdown complete");
        Ok(())
    }
}

/// Lifecycle errors
#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("Could not determine state directory")]
    NoStateDir,

    #[error("Failed to acquire lock: daemon already running?")]
    LockFailed(#[source] std::io::Error),

    #[error("Failed to bind socket at {0}: {1}")]
    BindFailed(PathBuf, std::io::Error),

    #[error("WAL error: {0}")]
    Wal(#[from] oj_storage::WalError),

    #[error("Snapshot error: {0}")]
    Snapshot(#[from] oj_storage::SnapshotError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Runtime error: {0}")]
    Runtime(String),
}

/// Start the daemon
pub async fn startup(config: &Config) -> Result<StartupResult, LifecycleError> {
    match startup_inner(config).await {
        Ok(result) => Ok(result),
        Err(e) => {
            // Don't clean up if we failed to acquire the lock —
            // those files belong to the already-running daemon.
            if !matches!(e, LifecycleError::LockFailed(_)) {
                cleanup_on_failure(config);
            }
            Err(e)
        }
    }
}

/// Inner startup logic - cleanup_on_failure called if this fails
async fn startup_inner(config: &Config) -> Result<StartupResult, LifecycleError> {
    // 1. Create state directory (needed for socket, lock, etc.)
    if let Some(parent) = config.socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // 2. Acquire lock file FIRST - prevents races
    // Use OpenOptions to avoid truncating the file before we hold the lock,
    // which would wipe the running daemon's PID.
    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&config.lock_path)?;
    lock_file
        .try_lock_exclusive()
        .map_err(LifecycleError::LockFailed)?;

    // Write PID to lock file (truncate now that we hold the lock)
    use std::io::Write;
    let mut lock_file = lock_file;
    lock_file.set_len(0)?;
    writeln!(lock_file, "{}", std::process::id())?;
    let lock_file = lock_file; // Drop mutability

    // 3. Create directories
    if let Some(parent) = config.socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = config.wal_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&config.workspaces_path)?;

    // Write version file
    std::fs::write(
        &config.version_path,
        concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH")),
    )?;

    // 4. Load state from snapshot (if exists) and replay Wal
    let (mut state, processed_seq) = match load_snapshot(&config.snapshot_path)? {
        Some(snapshot) => {
            info!(
                "Loaded snapshot at seq {}: {} jobs, {} sessions, {} workspaces",
                snapshot.seq,
                snapshot.state.jobs.len(),
                snapshot.state.sessions.len(),
                snapshot.state.workspaces.len()
            );
            (snapshot.state, snapshot.seq)
        }
        None => {
            info!("No snapshot found, starting with empty state");
            (MaterializedState::default(), 0)
        }
    };

    // Open Wal and create EventBus
    let event_wal = Wal::open(&config.wal_path, processed_seq)?;
    let events_to_replay = event_wal.entries_after(processed_seq)?;
    let (event_bus, event_reader) = EventBus::new(event_wal);
    let replay_count = events_to_replay.len();
    for entry in events_to_replay {
        state.apply_event(&entry.event);
    }

    if replay_count > 0 {
        info!(
            "Replayed {} events from WAL after seq {}",
            replay_count, processed_seq
        );
    }

    info!(
        "Recovered state: {} jobs, {} sessions, {} workspaces, {} agent_runs",
        state.jobs.len(),
        state.sessions.len(),
        state.workspaces.len(),
        state.agent_runs.len()
    );

    // 5. Set up adapters (wrapped with tracing for observability)
    let session_adapter = TracedSession::new(TmuxAdapter::new());
    // Set up agent log extraction channel
    let (log_entry_tx, log_entry_rx) = mpsc::channel(256);
    let agent_adapter = TracedAgent::new(
        ClaudeAgentAdapter::new(session_adapter.clone()).with_log_entry_tx(log_entry_tx),
    );

    // Spawn background task to write agent log entries
    AgentLogger::spawn_writer(config.logs_path.clone(), log_entry_rx);

    // 6. Create internal channel for runtime to emit events
    // Events from this channel will be forwarded to the EventBus
    let (internal_tx, internal_rx) = mpsc::channel::<Event>(100);
    spawn_runtime_event_forwarder(internal_rx, event_bus.clone());

    // 7. Remove stale socket and bind (LAST - only after all validation passes)
    if config.socket_path.exists() {
        std::fs::remove_file(&config.socket_path)?;
    }
    let listener = UnixListener::bind(&config.socket_path)
        .map_err(|e| LifecycleError::BindFailed(config.socket_path.clone(), e))?;

    // 7b. Detect orphaned jobs from breadcrumb files
    let breadcrumbs = breadcrumb::scan_breadcrumbs(&config.logs_path);
    let stale_threshold = std::time::Duration::from_secs(7 * 24 * 60 * 60); // 7 days
    let mut orphans = Vec::new();
    for bc in breadcrumbs {
        if let Some(job) = state.jobs.get(&bc.job_id) {
            // Job exists in recovered state — clean up stale breadcrumbs
            // for terminal jobs (crash between terminal and breadcrumb delete)
            if job.is_terminal() {
                let path = oj_engine::log_paths::breadcrumb_path(&config.logs_path, &bc.job_id);
                let _ = std::fs::remove_file(&path);
            }
        } else {
            // No matching job — check if breadcrumb is stale (> 7 days)
            let is_stale = {
                let path = oj_engine::log_paths::breadcrumb_path(&config.logs_path, &bc.job_id);
                match path.metadata() {
                    Ok(meta) => meta
                        .modified()
                        .ok()
                        .and_then(|mtime: std::time::SystemTime| mtime.elapsed().ok())
                        .map(|age| age > stale_threshold)
                        .unwrap_or(false),
                    Err(_) => false,
                }
            };
            if is_stale {
                warn!(
                    job_id = %bc.job_id,
                    "auto-dismissing stale orphan breadcrumb (> 7 days old)"
                );
                let path = oj_engine::log_paths::breadcrumb_path(&config.logs_path, &bc.job_id);
                let _ = std::fs::remove_file(&path);
            } else {
                warn!(
                    job_id = %bc.job_id,
                    project = %bc.project,
                    kind = %bc.kind,
                    step = %bc.current_step,
                    "orphaned job detected"
                );
                orphans.push(bc);
            }
        }
    }
    if !orphans.is_empty() {
        warn!(
            "{} orphaned job(s) detected from breadcrumbs",
            orphans.len()
        );
    }
    let orphans = Arc::new(Mutex::new(orphans));

    // 8. Wrap state in Arc<Mutex>
    let state = Arc::new(Mutex::new(state));

    // 9. Create runtime (runbook loaded on-demand per project)
    let runtime = Arc::new(Runtime::new(
        RuntimeDeps {
            sessions: session_adapter.clone(),
            agents: agent_adapter,
            notifier: DesktopNotifyAdapter::new(),
            state: Arc::clone(&state),
        },
        SystemClock,
        RuntimeConfig {
            state_dir: config.state_dir.clone(),
            log_dir: config.logs_path.clone(),
        },
        internal_tx.clone(),
    ));

    // 10. Spawn usage metrics collector
    let metrics_health = UsageMetricsCollector::spawn_collector(
        Arc::clone(&state),
        config.state_dir.join("metrics"),
    );

    // 11. Prepare reconciliation context (will run as background task after READY)
    //
    // Clone state to avoid holding the mutex during async reconciliation,
    // which also locks state internally (lock_state_mut) — holding the lock here
    // would deadlock.
    let state_snapshot = {
        let state_guard = state.lock();
        state_guard.clone()
    };
    let job_count = state_snapshot
        .jobs
        .values()
        .filter(|p| !p.is_terminal())
        .count();
    let worker_count = state_snapshot
        .workers
        .values()
        .filter(|w| w.status == "running")
        .count();
    let cron_count = state_snapshot
        .crons
        .values()
        .filter(|c| c.status == "running")
        .count();
    let agent_run_count = state_snapshot
        .agent_runs
        .values()
        .filter(|ar| !ar.is_terminal())
        .count();

    info!("Daemon started");

    Ok(StartupResult {
        daemon: DaemonState {
            config: config.clone(),
            lock_file,
            state,
            runtime: Arc::clone(&runtime),
            event_bus,
            start_time: Instant::now(),
            orphans,
            metrics_health,
        },
        listener,
        event_reader,
        reconcile_ctx: ReconcileCtx {
            runtime,
            state_snapshot,
            session_adapter,
            event_tx: internal_tx,
            job_count,
            worker_count,
            cron_count,
            agent_run_count,
        },
    })
}

/// Spawn task to forward runtime events to the event bus.
///
/// The runtime uses an mpsc channel internally. This task reads from that
/// channel and forwards events to the EventBus for durability.
///
/// After draining each batch of events, flushes the WAL to ensure durability.
/// This eliminates the 10ms group-commit window for engine-produced events,
/// making crash recovery reliable.
fn spawn_runtime_event_forwarder(mut rx: mpsc::Receiver<Event>, event_bus: EventBus) {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if event_bus.send(event).is_err() {
                tracing::warn!("Failed to forward runtime event to WAL");
                continue;
            }

            // Drain any additional buffered events before flushing
            while let Ok(event) = rx.try_recv() {
                if event_bus.send(event).is_err() {
                    tracing::warn!("Failed to forward runtime event to WAL");
                }
            }

            // Flush the batch to disk
            if let Err(e) = event_bus.flush() {
                tracing::error!("Failed to flush runtime events: {}", e);
            }
        }
    });
}

/// Clean up resources on startup failure
fn cleanup_on_failure(config: &Config) {
    // Remove socket if we created it
    if config.socket_path.exists() {
        let _ = std::fs::remove_file(&config.socket_path);
    }

    // Remove version file
    if config.version_path.exists() {
        let _ = std::fs::remove_file(&config.version_path);
    }

    // Remove PID/lock file
    if config.lock_path.exists() {
        let _ = std::fs::remove_file(&config.lock_path);
    }
}

/// Get the state directory for oj
fn state_dir() -> Result<PathBuf, LifecycleError> {
    crate::env::state_dir()
}

#[cfg(test)]
#[path = "../lifecycle_tests/mod.rs"]
mod tests;
