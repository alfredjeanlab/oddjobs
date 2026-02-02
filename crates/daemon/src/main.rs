// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Odd Jobs Daemon (ojd)
//!
//! Background process that owns the event loop and dispatches work.
//!
//! Architecture:
//! - Listener Task: Spawned task handling socket I/O, emits events to EventBus
//! - Engine Loop: Main thread processing events sequentially

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

mod event_bus;
mod lifecycle;
mod listener;
mod protocol;

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Duration;

use oj_core::{Clock, Event, PipelineId};
use oj_storage::{MaterializedState, Snapshot, Wal};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::Notify;
use tracing::{error, info};

use crate::event_bus::EventBus;
use crate::lifecycle::{Config, LifecycleError, StartupResult};
use crate::listener::Listener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Handle info flags before any config/lock acquisition
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "--version" | "-V" | "-v" => {
                println!(
                    "ojd {}",
                    concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH"))
                );
                return Ok(());
            }
            "--help" | "-h" | "help" => {
                println!(
                    "ojd {}",
                    concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH"))
                );
                println!("Odd Jobs Daemon - background process that owns the event loop and dispatches work");
                println!();
                println!("USAGE:");
                println!("    ojd");
                println!();
                println!("The daemon is typically started by the `oj` CLI and should not");
                println!("be invoked directly. It listens on a Unix socket for commands");
                println!("from `oj`.");
                println!();
                println!("OPTIONS:");
                println!("    -h, --help       Print help information");
                println!("    -v, --version    Print version information");
                return Ok(());
            }
            _ => {
                eprintln!("error: unexpected argument '{arg}'");
                eprintln!("Usage: ojd [--help | --version]");
                std::process::exit(1);
            }
        }
    }

    // Load configuration (user-level daemon, no project root)
    let config = Config::load()?;

    // Write startup marker to log (before tracing setup, so CLI can find it)
    write_startup_marker(&config)?;

    // Set up logging
    let log_guard = setup_logging(&config)?;

    info!("Starting user-level daemon");

    // Start daemon
    let StartupResult {
        mut daemon,
        listener: unix_listener,
        mut event_reader,
        reconcile_ctx,
    } = match lifecycle::startup(&config).await {
        Ok(r) => r,
        Err(LifecycleError::LockFailed(_)) => {
            // Another daemon is already running — print a human-readable message
            // instead of a raw debug error.
            let pid = std::fs::read_to_string(&config.lock_path)
                .unwrap_or_default()
                .trim()
                .to_string();
            let version = std::fs::read_to_string(&config.version_path)
                .unwrap_or_default()
                .trim()
                .to_string();

            eprintln!("ojd is already running");
            if !pid.is_empty() {
                eprintln!("  pid: {pid}");
            }
            if !version.is_empty() {
                let current_version =
                    concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH"));
                if version == current_version {
                    eprintln!("  version: {version}");
                } else {
                    eprintln!("  version: {version} (outdated — current: {current_version})");
                }
            }
            std::process::exit(1);
        }
        Err(e) => {
            // Write error synchronously (tracing is non-blocking and may not flush in time)
            write_startup_error(&config, &e);
            error!("Failed to start daemon: {}", e);
            drop(log_guard);
            return Err(e.into());
        }
    };

    // Shutdown signal: non-durable channel so shutdown requests are not
    // persisted to the WAL and accidentally replayed on next startup.
    let shutdown_notify = Arc::new(Notify::new());

    // Spawn listener task
    let listener = Listener::new(
        unix_listener,
        daemon.event_bus.clone(),
        Arc::clone(&daemon.state),
        daemon.config.logs_path.clone(),
        daemon.start_time,
        Arc::clone(&shutdown_notify),
    );
    tokio::spawn(listener.run());

    // Spawn checkpoint task for periodic snapshots
    spawn_checkpoint(
        Arc::clone(&daemon.state),
        event_reader.wal(),
        daemon.config.snapshot_path.clone(),
    );

    // Spawn flush task for group commit (~10ms durability window)
    spawn_flush_task(daemon.event_bus.clone());

    // Set up signal handlers
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    info!(
        "Daemon ready, listening on {}",
        config.socket_path.display()
    );

    // Signal ready for parent process (e.g., systemd, CLI waiting for startup)
    println!("READY");

    // Spawn background reconciliation — daemon is already accepting connections
    if reconcile_ctx.pipeline_count > 0 || reconcile_ctx.worker_count > 0 {
        info!(
            "spawning background reconciliation for {} pipelines and {} workers",
            reconcile_ctx.pipeline_count, reconcile_ctx.worker_count
        );
        tokio::spawn(async move {
            lifecycle::reconcile_state(
                &reconcile_ctx.runtime,
                &reconcile_ctx.state_snapshot,
                &reconcile_ctx.session_adapter,
                &reconcile_ctx.event_tx,
            )
            .await;
            info!("background reconciliation complete");
        });
    } else {
        drop(reconcile_ctx); // Nothing to reconcile
    }

    // Timer check interval (1-second resolution)
    // NOTE: Must be created outside the loop - tokio::select! re-evaluates
    // branches on each iteration, so using sleep() inside would reset on
    // every event, causing timers to never fire during activity.
    let mut timer_check = tokio::time::interval(Duration::from_secs(1));

    // Engine loop - processes events sequentially from WAL
    loop {
        tokio::select! {
            // Process events from the durable event reader
            result = event_reader.recv() => {
                match result {
                    Ok(Some(entry)) => {
                        let seq = entry.seq;
                        match entry.event {
                            Event::Shutdown => {
                                // Skip shutdown events from WAL - they are
                                // control signals that must not be replayed on restart.
                                event_reader.mark_processed(seq);
                            }
                            event => {
                                let pipeline_id = event.pipeline_id().map(|p| p.to_string());
                                let is_failure = matches!(
                                    &event,
                                    Event::PipelineAdvanced { step, .. } if step == "failed"
                                );
                                match daemon.process_event(event).await {
                                    Ok(()) => event_reader.mark_processed(seq),
                                    Err(e) => {
                                        // Mark processed - unprocessable events must not
                                        // block the event loop. If an event can't be
                                        // processed now, it won't be processable later.
                                        error!("Error processing event (seq={}): {}", seq, e);
                                        event_reader.mark_processed(seq);

                                        // Best-effort: fail the associated pipeline so it
                                        // doesn't get stuck. Skip if already a failure
                                        // transition to avoid cascading events.
                                        if let Some(pid) = pipeline_id.filter(|_| !is_failure) {
                                            let fail_event = Event::PipelineAdvanced {
                                                id: PipelineId::new(pid),
                                                step: "failed".to_string(),
                                            };
                                            if let Err(send_err) = daemon.event_bus.send(fail_event) {
                                                error!("Failed to emit pipeline failure event: {}", send_err);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        info!("Event bus closed, shutting down...");
                        break;
                    }
                    Err(e) => {
                        error!("Error reading from WAL: {}", e);
                    }
                }
            }

            // Shutdown requested via command
            _ = shutdown_notify.notified() => {
                info!("Shutdown requested via command");
                break;
            }

            // Graceful shutdown on SIGTERM
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down...");
                break;
            }

            // Graceful shutdown on SIGINT
            _ = sigint.recv() => {
                info!("Received SIGINT, shutting down...");
                break;
            }

            // Fire timers periodically (1-second resolution)
            _ = timer_check.tick() => {
                let now = daemon.runtime.clock().now();
                let timer_events = {
                    let mut scheduler = daemon.scheduler.lock();
                    scheduler.fired_timers(now)
                };
                for event in timer_events {
                    if let Err(e) = daemon.event_bus.send(event) {
                        error!("Failed to send timer event: {}", e);
                    }
                }
            }
        }
    }

    // Graceful shutdown (session kills already handled in listener for --kill)
    daemon.shutdown()?;
    info!("Daemon stopped");
    Ok(())
}

/// Flush interval for group commit (~10ms durability window)
const FLUSH_INTERVAL: Duration = Duration::from_millis(10);

/// Spawn a task that periodically flushes the event bus.
fn spawn_flush_task(event_bus: EventBus) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(FLUSH_INTERVAL);

        loop {
            interval.tick().await;

            if event_bus.needs_flush() {
                if let Err(e) = event_bus.flush() {
                    tracing::error!("Failed to flush event bus: {}", e);
                }
            }
        }
    });
}

/// Checkpoint interval (60 seconds)
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(60);

/// Spawn a task that periodically saves snapshots and truncates WAL.
///
/// This provides durability with bounded recovery time.
fn spawn_checkpoint(
    state: Arc<Mutex<MaterializedState>>,
    event_wal: Arc<Mutex<Wal>>,
    snapshot_path: PathBuf,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECKPOINT_INTERVAL);

        loop {
            interval.tick().await;

            // Get current state and processed seq
            let (state_clone, processed_seq) = {
                let state_guard = state.lock();
                let wal_guard = event_wal.lock();
                (state_guard.clone(), wal_guard.processed_seq())
            };

            // Only checkpoint if we've processed some events
            if processed_seq == 0 {
                continue;
            }

            // Save snapshot
            let snapshot = Snapshot::new(processed_seq, state_clone);
            match snapshot.save(&snapshot_path) {
                Ok(()) => {
                    tracing::debug!(seq = processed_seq, "saved checkpoint snapshot");

                    // Truncate WAL entries before snapshot
                    let mut wal = event_wal.lock();
                    if let Err(e) = wal.truncate_before(processed_seq) {
                        tracing::warn!(
                            error = %e,
                            "failed to truncate WAL after checkpoint"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "failed to save checkpoint snapshot"
                    );
                }
            }
        }
    });
}

/// Startup marker prefix written to log before anything else.
/// CLI uses this to find where the current startup attempt begins.
/// Full format: "--- ojd: starting (pid: 12345) ---"
pub const STARTUP_MARKER_PREFIX: &str = "--- ojd: starting (pid: ";

/// Write startup marker to log file (appends to existing log)
fn write_startup_marker(config: &Config) -> Result<(), LifecycleError> {
    use std::io::Write;

    // Create log directory if needed
    if let Some(parent) = config.log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Append marker to log file with PID
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.log_path)?;
    writeln!(file, "{}{})", STARTUP_MARKER_PREFIX, std::process::id())?;

    Ok(())
}

/// Write startup error synchronously to log file.
/// This ensures the error is visible to the CLI even if the process exits quickly.
fn write_startup_error(config: &Config, error: &LifecycleError) {
    use std::io::Write;

    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.log_path)
    else {
        return;
    };
    let _ = writeln!(file, "ERROR Failed to start daemon: {}", error);
}

fn setup_logging(
    config: &Config,
) -> Result<tracing_appender::non_blocking::WorkerGuard, LifecycleError> {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    // Create log directory if needed
    if let Some(parent) = config.log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Set up file appender
    let file_appender = tracing_appender::rolling::never(
        config.log_path.parent().ok_or(LifecycleError::NoStateDir)?,
        config
            .log_path
            .file_name()
            .ok_or(LifecycleError::NoStateDir)?,
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Set up subscriber with env filter
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(non_blocking))
        .init();

    Ok(guard)
}
