// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Listener task for handling socket I/O.
//!
//! The Listener runs in a spawned task, accepting connections and
//! handling them without blocking the engine loop. Events are emitted
//! onto the EventBus for processing by the engine.

mod commands;
mod crons;
mod decisions;
mod mutations;
mod query;
mod queues;
mod suggest;
mod tmux;
mod workers;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Instant;

use oj_core::Event;
use oj_storage::MaterializedState;
use thiserror::Error;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
use tracing::{debug, error, warn};

use crate::event_bus::EventBus;
use oj_engine::breadcrumb::Breadcrumb;

use crate::protocol::{self, Request, Response, DEFAULT_TIMEOUT, PROTOCOL_VERSION};

/// Listener task for accepting socket connections.
pub(crate) struct Listener {
    socket: UnixListener,
    event_bus: EventBus,
    state: Arc<Mutex<MaterializedState>>,
    orphans: Arc<Mutex<Vec<Breadcrumb>>>,
    logs_path: std::path::PathBuf,
    start_time: Instant,
    shutdown: Arc<Notify>,
}

/// Errors from connection handling.
#[derive(Debug, Error)]
pub(crate) enum ConnectionError {
    #[error("Protocol error: {0}")]
    Protocol(#[from] protocol::ProtocolError),

    #[error("WAL error")]
    WalError,

    #[error("Internal error: {0}")]
    Internal(String),
}

impl Listener {
    /// Create a new listener.
    pub fn new(
        socket: UnixListener,
        event_bus: EventBus,
        state: Arc<Mutex<MaterializedState>>,
        orphans: Arc<Mutex<Vec<Breadcrumb>>>,
        logs_path: std::path::PathBuf,
        start_time: Instant,
        shutdown: Arc<Notify>,
    ) -> Self {
        Self {
            socket,
            event_bus,
            state,
            orphans,
            logs_path,
            start_time,
            shutdown,
        }
    }

    /// Run the listener loop until shutdown, spawning tasks for each connection.
    pub async fn run(self) {
        loop {
            match self.socket.accept().await {
                Ok((stream, _)) => {
                    let event_bus = self.event_bus.clone();
                    let state = Arc::clone(&self.state);
                    let orphans = Arc::clone(&self.orphans);
                    let logs_path = self.logs_path.clone();
                    let start_time = self.start_time;
                    let shutdown = Arc::clone(&self.shutdown);

                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(
                            stream, event_bus, state, orphans, logs_path, start_time, shutdown,
                        )
                        .await
                        {
                            match e {
                                ConnectionError::Protocol(
                                    protocol::ProtocolError::ConnectionClosed,
                                ) => debug!("Client disconnected"),
                                ConnectionError::Protocol(protocol::ProtocolError::Timeout) => {
                                    warn!("Connection timeout")
                                }
                                _ => error!("Connection error: {}", e),
                            }
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }
}

/// Handle a single client connection.
async fn handle_connection(
    stream: UnixStream,
    event_bus: EventBus,
    state: Arc<Mutex<MaterializedState>>,
    orphans: Arc<Mutex<Vec<Breadcrumb>>>,
    logs_path: std::path::PathBuf,
    start_time: Instant,
    shutdown: Arc<Notify>,
) -> Result<(), ConnectionError> {
    let (mut reader, mut writer) = stream.into_split();

    // Read request with timeout
    let request = protocol::read_request(&mut reader, DEFAULT_TIMEOUT).await?;

    // Log queries at debug level (frequent polling), other requests at info
    if matches!(request, Request::Query { .. }) {
        debug!(request = ?request, "received query");
    } else {
        tracing::info!(request = ?request, "received request");
    }

    // Handle request
    let response = handle_request(
        request, &event_bus, &state, &orphans, &logs_path, start_time, &shutdown,
    )
    .await?;

    debug!("Sending response: {:?}", response);

    // Write response with timeout
    protocol::write_response(&mut writer, &response, DEFAULT_TIMEOUT).await?;

    Ok(())
}

/// Handle a single request and return a response.
async fn handle_request(
    request: Request,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    logs_path: &std::path::Path,
    start_time: Instant,
    shutdown: &Notify,
) -> Result<Response, ConnectionError> {
    match request {
        Request::Ping => Ok(Response::Pong),

        Request::Hello { version: _ } => Ok(Response::Hello {
            version: PROTOCOL_VERSION.to_string(),
        }),

        Request::Event { event } => {
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;
            Ok(Response::Ok)
        }

        Request::Query { query } => Ok(query::handle_query(
            query, state, orphans, logs_path, start_time,
        )),

        Request::Shutdown { kill } => {
            if kill {
                tmux::kill_state_sessions(state).await;
            }
            shutdown.notify_one();
            Ok(Response::ShuttingDown)
        }

        Request::Status => Ok(mutations::handle_status(state, orphans, start_time)),

        Request::SessionSend { id, input } => {
            mutations::handle_session_send(state, event_bus, id, input)
        }

        Request::SessionKill { id } => mutations::handle_session_kill(state, event_bus, &id).await,

        Request::AgentSend { agent_id, message } => {
            mutations::handle_agent_send(state, event_bus, agent_id, message).await
        }

        Request::JobResume { id, message, vars } => {
            mutations::handle_job_resume(state, orphans, event_bus, id, message, vars)
        }

        Request::JobCancel { ids } => mutations::handle_job_cancel(state, event_bus, ids),

        Request::RunCommand {
            project_root,
            invoke_dir,
            namespace,
            command,
            args,
            named_args,
        } => {
            commands::handle_run_command(commands::RunCommandParams {
                project_root: &project_root,
                invoke_dir: &invoke_dir,
                namespace: &namespace,
                command: &command,
                args: &args,
                named_args: &named_args,
                event_bus,
                state,
            })
            .await
        }

        Request::PeekSession {
            session_id,
            with_color,
        } => match tmux::capture_tmux_pane(&session_id, with_color).await {
            Ok(output) => Ok(Response::SessionPeek { output }),
            Err(message) => Ok(Response::Error { message }),
        },

        Request::WorkspaceDrop { id } => {
            mutations::handle_workspace_drop(state, event_bus, Some(&id), false, false).await
        }

        Request::WorkspaceDropFailed => {
            mutations::handle_workspace_drop(state, event_bus, None, true, false).await
        }

        Request::WorkspaceDropAll => {
            mutations::handle_workspace_drop(state, event_bus, None, false, true).await
        }

        Request::JobPrune {
            all,
            failed,
            orphans: prune_orphans,
            dry_run,
            namespace,
        } => {
            let flags = mutations::PruneFlags {
                all,
                dry_run,
                namespace: namespace.as_deref(),
            };
            mutations::handle_job_prune(
                state,
                event_bus,
                logs_path,
                orphans,
                &flags,
                failed,
                prune_orphans,
            )
        }

        Request::AgentPrune { all, dry_run } => {
            let flags = mutations::PruneFlags {
                all,
                dry_run,
                namespace: None,
            };
            mutations::handle_agent_prune(state, event_bus, logs_path, &flags)
        }

        Request::WorkspacePrune {
            all,
            dry_run,
            namespace,
        } => {
            let flags = mutations::PruneFlags {
                all,
                dry_run,
                namespace: namespace.as_deref(),
            };
            mutations::handle_workspace_prune(state, event_bus, &flags).await
        }

        Request::WorkerPrune {
            all,
            dry_run,
            namespace,
        } => {
            let flags = mutations::PruneFlags {
                all,
                dry_run,
                namespace: namespace.as_deref(),
            };
            mutations::handle_worker_prune(state, event_bus, &flags)
        }

        Request::CronPrune { all, dry_run } => {
            let flags = mutations::PruneFlags {
                all,
                dry_run,
                namespace: None,
            };
            mutations::handle_cron_prune(state, event_bus, &flags)
        }

        Request::WorkerStart {
            project_root,
            namespace,
            worker_name,
        } => {
            workers::handle_worker_start(&project_root, &namespace, &worker_name, event_bus, state)
        }

        Request::WorkerWake {
            worker_name,
            namespace,
        } => {
            // Internal-only: emits WorkerWake event for queue auto-wake.
            // CLI uses WorkerStart (idempotent) instead.
            let event = Event::WorkerWake {
                worker_name,
                namespace,
            };
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;
            Ok(Response::Ok)
        }

        Request::WorkerStop {
            worker_name,
            namespace,
            project_root,
        } => workers::handle_worker_stop(
            &worker_name,
            &namespace,
            event_bus,
            state,
            project_root.as_deref(),
        ),

        Request::WorkerRestart {
            project_root,
            namespace,
            worker_name,
        } => workers::handle_worker_restart(
            &project_root,
            &namespace,
            &worker_name,
            event_bus,
            state,
        ),

        Request::CronStart {
            project_root,
            namespace,
            cron_name,
        } => crons::handle_cron_start(&project_root, &namespace, &cron_name, event_bus, state),

        Request::CronStop {
            cron_name,
            namespace,
            project_root,
        } => crons::handle_cron_stop(
            &cron_name,
            &namespace,
            event_bus,
            state,
            project_root.as_deref(),
        ),

        Request::CronRestart {
            project_root,
            namespace,
            cron_name,
        } => crons::handle_cron_restart(&project_root, &namespace, &cron_name, event_bus, state),

        Request::CronOnce {
            project_root,
            namespace,
            cron_name,
        } => crons::handle_cron_once(&project_root, &namespace, &cron_name, event_bus, state).await,

        Request::QueuePush {
            project_root,
            namespace,
            queue_name,
            data,
        } => queues::handle_queue_push(
            &project_root,
            &namespace,
            &queue_name,
            data,
            event_bus,
            state,
        ),

        Request::QueueDrop {
            project_root,
            namespace,
            queue_name,
            item_id,
        } => queues::handle_queue_drop(
            &project_root,
            &namespace,
            &queue_name,
            &item_id,
            event_bus,
            state,
        ),

        Request::QueueRetry {
            project_root,
            namespace,
            queue_name,
            item_id,
        } => queues::handle_queue_retry(
            &project_root,
            &namespace,
            &queue_name,
            &item_id,
            event_bus,
            state,
        ),

        Request::QueueDrain {
            project_root,
            namespace,
            queue_name,
        } => queues::handle_queue_drain(&project_root, &namespace, &queue_name, event_bus, state),

        Request::QueueFail {
            project_root,
            namespace,
            queue_name,
            item_id,
        } => queues::handle_queue_fail(
            &project_root,
            &namespace,
            &queue_name,
            &item_id,
            event_bus,
            state,
        ),

        Request::QueueDone {
            project_root,
            namespace,
            queue_name,
            item_id,
        } => queues::handle_queue_done(
            &project_root,
            &namespace,
            &queue_name,
            &item_id,
            event_bus,
            state,
        ),

        Request::QueuePrune {
            project_root,
            namespace,
            queue_name,
            all,
            dry_run,
        } => queues::handle_queue_prune(
            &project_root,
            &namespace,
            &queue_name,
            all,
            dry_run,
            event_bus,
            state,
        ),

        Request::DecisionResolve {
            id,
            chosen,
            message,
        } => decisions::handle_decision_resolve(&id, chosen, message, event_bus, state),

        Request::AgentResume {
            agent_id,
            kill,
            all,
        } => mutations::handle_agent_resume(state, event_bus, agent_id, kill, all).await,
    }
}

/// Load a runbook, falling back to the known project root for the namespace.
///
/// Tries `load_fn(project_root)` first; if that fails, looks up the known root
/// for the namespace in materialized state (supports `--project` from a different
/// directory). On total failure, calls `suggest_fn` to generate a "did you mean" hint.
fn load_runbook_with_fallback(
    project_root: &Path,
    namespace: &str,
    state: &Arc<Mutex<MaterializedState>>,
    load_fn: impl Fn(&Path) -> Result<oj_runbook::Runbook, String>,
    suggest_fn: impl FnOnce() -> String,
) -> Result<(oj_runbook::Runbook, PathBuf), Response> {
    match load_fn(project_root) {
        Ok(rb) => Ok((rb, project_root.to_path_buf())),
        Err(e) => {
            let known_root = {
                let st = state.lock();
                st.project_root_for_namespace(namespace)
            };
            let alt_result = known_root
                .as_deref()
                .filter(|alt| *alt != project_root)
                .and_then(|alt| load_fn(alt).ok().map(|rb| (rb, alt.to_path_buf())));
            match alt_result {
                Some(result) => Ok(result),
                None => {
                    let hint = suggest_fn();
                    Err(Response::Error {
                        message: format!("{}{}", e, hint),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../listener_tests.rs"]
mod tests;
