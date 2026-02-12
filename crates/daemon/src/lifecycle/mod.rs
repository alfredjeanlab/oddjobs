// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Daemon lifecycle management: startup, shutdown, recovery.

mod reconcile;
mod startup;
pub(crate) use reconcile::reconcile_state;
pub use startup::startup;

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Instant;

use oj_adapters::{DesktopNotifyAdapter, RuntimeRouter};
use oj_core::{Event, SystemClock};
use oj_engine::breadcrumb::Breadcrumb;
use oj_engine::{MetricsHealth, Runtime};
use oj_storage::{Checkpointer, MaterializedState};
use thiserror::Error;
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::event_bus::{EventBus, EventReader};

/// Daemon runtime with concrete adapter types
pub type DaemonRuntime = Runtime<RuntimeRouter, DesktopNotifyAdapter, SystemClock>;

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
    /// Channel for emitting events discovered during reconciliation
    pub event_tx: mpsc::Sender<Event>,
    /// Root state directory for coop socket discovery
    pub state_dir: PathBuf,
    /// Number of non-terminal jobs to reconcile
    pub job_count: usize,
    /// Number of workers with status "running" to reconcile
    pub worker_count: usize,
    /// Number of crons with status "running" to reconcile
    pub cron_count: usize,
    /// Number of non-terminal crew to reconcile
    pub crew_count: usize,
}

impl DaemonState {
    /// Process an event through the runtime.
    ///
    /// Result events are persisted to the WAL and will be processed by the
    /// engine loop on the next iteration. We deliberately do NOT process them
    /// locally to avoid double-delivery: the engine loop already reads every
    /// WAL entry exactly once, so processing here as well would cause handlers
    /// to fire twice (e.g. duplicate job creation from WorkerPolled).
    pub async fn process_event(&mut self, event: Event) -> Result<(), LifecycleError> {
        // For deletion events, run the handler BEFORE state mutation so
        // cleanup code can read the data that's about to be removed.
        // handle_job_deleted needs job step_history, workspace_id.
        let pre_delete_events = if matches!(&event, Event::JobDeleted { .. }) {
            self.runtime
                .handle_event(event.clone())
                .await
                .map_err(|e| LifecycleError::Runtime(e.to_string()))?
        } else {
            vec![]
        };

        // Apply the incoming event to materialized state so queries see it.
        // (Effect::Emit events are also applied in the executor for immediate
        // visibility; apply_event is idempotent so the second apply when those
        // events return from the WAL is harmless.)
        {
            let mut state = self.state.lock();
            state.apply_event(&event);
        }

        // Handle non-deletion events normally (deletion already handled above)
        let result_events = if matches!(&event, Event::JobDeleted { .. }) {
            vec![]
        } else {
            self.runtime
                .handle_event(event)
                .await
                .map_err(|e| LifecycleError::Runtime(e.to_string()))?
        };

        // Persist all result events to WAL — the engine loop will read and process
        // them on the next iteration, ensuring single delivery.
        for result_event in pre_delete_events.into_iter().chain(result_events) {
            if let Err(e) = self.event_bus.send(result_event) {
                warn!("Failed to persist runtime result event to WAL: {}", e);
            }
        }

        Ok(())
    }

    /// Shutdown the daemon gracefully.
    ///
    /// Agent processes are intentionally preserved across daemon restarts so that
    /// long-running agents continue processing. On next startup, `reconcile_state`
    /// reconnects to surviving agents. Use `Request::Shutdown { kill: true }` to
    /// terminate all agent sessions before stopping (handled in the listener
    /// before the shutdown signal is sent, so that kills complete before the
    /// CLI starts its exit timer).
    pub fn shutdown(&mut self) -> Result<(), LifecycleError> {
        info!("Shutting down daemon...");

        // 0. Flush buffered WAL events to disk before tearing down
        if let Err(e) = self.event_bus.wal.lock().flush() {
            warn!("Failed to flush WAL on shutdown: {}", e);
        }

        // 0b. Save final snapshot so next startup doesn't need to replay WAL
        // Uses synchronous checkpoint with compression for fast subsequent startup
        let processed_seq = self.event_bus.wal.lock().processed_seq();
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

/// Get the state directory for oj
fn state_dir() -> Result<PathBuf, LifecycleError> {
    crate::env::state_dir()
}

#[cfg(test)]
mod test_helpers;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
