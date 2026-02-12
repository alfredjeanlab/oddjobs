// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Daemon startup and initialization logic.

use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

use crate::adapters::{DesktopNotifyAdapter, RuntimeRouter};
use crate::engine::breadcrumb;
use crate::engine::{AgentLogger, Runtime, RuntimeConfig, RuntimeDeps, UsageMetricsCollector};
use crate::storage::{load_snapshot, MaterializedState, Wal};
use fs2::FileExt;
use oj_core::{Event, JobId, SystemClock};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::event_bus::EventBus;

use super::{Config, DaemonState, LifecycleError, ReconcileCtx, StartupResult};

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
    lock_file.try_lock_exclusive().map_err(LifecycleError::LockFailed)?;

    // Write PID to lock file (truncate now that we hold the lock)
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
                "Loaded snapshot at seq {}: {} jobs, {} workspaces",
                snapshot.seq,
                snapshot.state.jobs.len(),
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
        info!("Replayed {} events from WAL after seq {}", replay_count, processed_seq);
    }

    info!(
        "Recovered state: {} jobs, {} workspaces, {} crew",
        state.jobs.len(),
        state.workspaces.len(),
        state.crew.len()
    );

    // 5. Set up adapters
    // Set up agent log extraction channel
    let (log_entry_tx, log_entry_rx) = mpsc::channel(256);
    let agent_adapter =
        RuntimeRouter::new(config.state_dir.clone()).with_log_entry_tx(log_entry_tx);

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
    let orphans = Vec::new();
    for bc in breadcrumbs {
        if let Some(job) = state.jobs.get(&bc.job_id) {
            // Job exists in recovered state — clean up stale breadcrumbs
            // for terminal jobs (crash between terminal and breadcrumb delete)
            if job.is_terminal() {
                let path = oj_core::log_paths::breadcrumb_path(&config.logs_path, &bc.job_id);
                let _ = std::fs::remove_file(&path);
            }
        } else {
            // No matching job — check if breadcrumb is stale (> 7 days)
            let is_stale = {
                let path = oj_core::log_paths::breadcrumb_path(&config.logs_path, &bc.job_id);
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
                warn!(job_id = %bc.job_id, "auto-dismissing stale orphan breadcrumb (> 7 days old)");
                let path = oj_core::log_paths::breadcrumb_path(&config.logs_path, &bc.job_id);
                let _ = std::fs::remove_file(&path);
            } else {
                // Emit synthetic events to fail the orphaned job so it
                // appears as "failed" in state rather than being invisible.
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let job_id = JobId(bc.job_id.clone());
                let mut events = vec![
                    Event::JobCreated {
                        id: job_id.clone(),
                        kind: bc.kind.clone(),
                        name: bc.name.clone(),
                        runbook_hash: bc.runbook_hash.clone(),
                        cwd: bc.cwd.clone().unwrap_or_else(|| std::path::PathBuf::from("/")),
                        vars: bc.vars.clone(),
                        initial_step: bc.current_step.clone(),
                        created_at_ms: now_ms,
                        project: bc.project.clone(),
                        cron: None,
                    },
                    Event::JobAdvanced { id: job_id, step: "failed".to_string() },
                ];

                // For shell-based orphans (no agents), mark the queue item
                // as dead so it isn't re-dispatched (the shell process died
                // with the daemon and re-running it is not safe).
                // Agent-based orphans are left for reconciliation which can
                // invoke on_dead handlers from the runbook.
                if bc.agents.is_empty() {
                    if let Some(item_id) = bc.name.strip_prefix(&format!("{}-", bc.kind)) {
                        for items in state.queue_items.values() {
                            if let Some(item) = items.iter().find(|i| i.id == item_id) {
                                events.push(Event::QueueFailed {
                                    queue: item.queue.clone(),
                                    item_id: item_id.to_string(),
                                    error: "job orphaned after daemon crash".to_string(),
                                    project: bc.project.clone(),
                                });
                                events.push(Event::QueueDead {
                                    queue: item.queue.clone(),
                                    item_id: item_id.to_string(),
                                    project: bc.project.clone(),
                                });
                                break;
                            }
                        }
                    }
                }

                for event in &events {
                    state.apply_event(event);
                }
                for event in events {
                    event_bus.send(event)?;
                }

                info!(job_id = %bc.job_id, project = %bc.project, "failed orphaned job from breadcrumb");

                // Delete breadcrumb now that the job is properly failed
                let path = oj_core::log_paths::breadcrumb_path(&config.logs_path, &bc.job_id);
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    // Flush synthetic orphan events to WAL before continuing
    event_bus.wal.lock().flush()?;

    if !orphans.is_empty() {
        warn!("{} orphaned job(s) detected from breadcrumbs", orphans.len());
    }
    let orphans = Arc::new(Mutex::new(orphans));

    // 8. Wrap state in Arc<Mutex>
    let state = Arc::new(Mutex::new(state));

    // 9. Create runtime (runbook loaded on-demand per project)
    let runtime = Arc::new(Runtime::new(
        RuntimeDeps {
            agents: agent_adapter.clone(),
            notifier: DesktopNotifyAdapter::new(),
            state: Arc::clone(&state),
        },
        SystemClock,
        RuntimeConfig { state_dir: config.state_dir.clone(), log_dir: config.logs_path.clone() },
        internal_tx.clone(),
    ));

    // 10. Spawn usage metrics collector
    let metrics_health = UsageMetricsCollector::spawn_collector(
        Arc::clone(&state),
        agent_adapter.clone(),
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
    let job_count = state_snapshot.jobs.values().filter(|p| !p.is_terminal()).count();
    let worker_count = state_snapshot.workers.values().filter(|w| w.status == "running").count();
    let cron_count = state_snapshot.crons.values().filter(|c| c.status == "running").count();
    let crew_count = state_snapshot.crew.values().filter(|run| !run.status.is_terminal()).count();

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
            event_tx: internal_tx,
            state_dir: config.state_dir.clone(),
            job_count,
            worker_count,
            cron_count,
            crew_count,
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
            if let Err(e) = event_bus.wal.lock().flush() {
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

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
