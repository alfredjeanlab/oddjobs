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
use oj_engine::MetricsHealth;

use crate::protocol::{self, Request, Response, DEFAULT_TIMEOUT, PROTOCOL_VERSION};

/// Shared daemon context for all request handlers.
pub(crate) struct ListenCtx {
    pub event_bus: EventBus,
    pub state: Arc<Mutex<MaterializedState>>,
    pub orphans: Arc<Mutex<Vec<Breadcrumb>>>,
    pub metrics_health: Arc<Mutex<MetricsHealth>>,
    pub logs_path: PathBuf,
    pub start_time: Instant,
    pub shutdown: Arc<Notify>,
}

/// Listener task for accepting socket connections.
pub(crate) struct Listener {
    socket: UnixListener,
    ctx: Arc<ListenCtx>,
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
    pub fn new(socket: UnixListener, ctx: Arc<ListenCtx>) -> Self {
        Self { socket, ctx }
    }

    /// Run the listener loop until shutdown, spawning tasks for each connection.
    pub async fn run(self) {
        loop {
            match self.socket.accept().await {
                Ok((stream, _)) => {
                    let ctx = Arc::clone(&self.ctx);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, &ctx).await {
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
async fn handle_connection(stream: UnixStream, ctx: &ListenCtx) -> Result<(), ConnectionError> {
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
    let response = handle_request(request, ctx).await?;

    debug!("Sending response: {:?}", response);

    // Write response with timeout
    protocol::write_response(&mut writer, &response, DEFAULT_TIMEOUT).await?;

    Ok(())
}

/// Handle a single request and return a response.
async fn handle_request(request: Request, ctx: &ListenCtx) -> Result<Response, ConnectionError> {
    match request {
        Request::Ping => Ok(Response::Pong),

        Request::Hello { version: _ } => Ok(Response::Hello {
            version: PROTOCOL_VERSION.to_string(),
        }),

        Request::Event { event } => {
            mutations::emit(&ctx.event_bus, event)?;
            Ok(Response::Ok)
        }

        Request::Query { query } => Ok(query::handle_query(ctx, query)),

        Request::Shutdown { kill } => {
            if kill {
                tmux::kill_state_sessions(&ctx.state).await;
            }
            ctx.shutdown.notify_one();
            Ok(Response::ShuttingDown)
        }

        Request::Status => Ok(mutations::handle_status(ctx)),

        Request::SessionSend { id, input } => mutations::handle_session_send(ctx, id, input),

        Request::SessionKill { id } => mutations::handle_session_kill(ctx, &id).await,

        Request::AgentSend { agent_id, message } => {
            mutations::handle_agent_send(ctx, agent_id, message).await
        }

        Request::JobResume {
            id,
            message,
            vars,
            kill,
            all,
        } => {
            if all {
                mutations::handle_job_resume_all(ctx, kill)
            } else {
                mutations::handle_job_resume(ctx, id, message, vars, kill)
            }
        }

        Request::JobResumeAll { kill } => mutations::handle_job_resume_all(ctx, kill),

        Request::JobCancel { ids } => mutations::handle_job_cancel(ctx, ids),

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
                ctx,
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
            mutations::handle_workspace_drop(ctx, Some(&id), false, false).await
        }

        Request::WorkspaceDropFailed => {
            mutations::handle_workspace_drop(ctx, None, true, false).await
        }

        Request::WorkspaceDropAll => mutations::handle_workspace_drop(ctx, None, false, true).await,

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
            mutations::handle_job_prune(ctx, &flags, failed, prune_orphans)
        }

        Request::AgentPrune { all, dry_run } => {
            let flags = mutations::PruneFlags {
                all,
                dry_run,
                namespace: None,
            };
            mutations::handle_agent_prune(ctx, &flags)
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
            mutations::handle_workspace_prune(ctx, &flags).await
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
            mutations::handle_worker_prune(ctx, &flags)
        }

        Request::CronPrune { all, dry_run } => {
            let flags = mutations::PruneFlags {
                all,
                dry_run,
                namespace: None,
            };
            mutations::handle_cron_prune(ctx, &flags)
        }

        Request::WorkerStart {
            project_root,
            namespace,
            worker_name,
            all,
        } => workers::handle_worker_start(ctx, &project_root, &namespace, &worker_name, all),

        Request::WorkerWake {
            worker_name,
            namespace,
        } => {
            mutations::emit(
                &ctx.event_bus,
                Event::WorkerWake {
                    worker_name,
                    namespace,
                },
            )?;
            Ok(Response::Ok)
        }

        Request::WorkerStop {
            worker_name,
            namespace,
            project_root,
            all,
        } => {
            workers::handle_worker_stop(ctx, &worker_name, &namespace, project_root.as_deref(), all)
        }

        Request::WorkerRestart {
            project_root,
            namespace,
            worker_name,
        } => workers::handle_worker_restart(ctx, &project_root, &namespace, &worker_name),

        Request::WorkerResize {
            worker_name,
            namespace,
            concurrency,
        } => workers::handle_worker_resize(ctx, &worker_name, &namespace, concurrency),

        Request::CronStart {
            project_root,
            namespace,
            cron_name,
            all,
        } => crons::handle_cron_start(ctx, &project_root, &namespace, &cron_name, all),

        Request::CronStop {
            cron_name,
            namespace,
            project_root,
        } => crons::handle_cron_stop(ctx, &cron_name, &namespace, project_root.as_deref()),

        Request::CronRestart {
            project_root,
            namespace,
            cron_name,
        } => crons::handle_cron_restart(ctx, &project_root, &namespace, &cron_name),

        Request::CronOnce {
            project_root,
            namespace,
            cron_name,
        } => crons::handle_cron_once(ctx, &project_root, &namespace, &cron_name).await,

        Request::QueuePush {
            project_root,
            namespace,
            queue_name,
            data,
        } => queues::handle_queue_push(ctx, &project_root, &namespace, &queue_name, data),

        Request::QueueDrop {
            project_root,
            namespace,
            queue_name,
            item_id,
        } => queues::handle_queue_drop(ctx, &project_root, &namespace, &queue_name, &item_id),

        Request::QueueRetry {
            project_root,
            namespace,
            queue_name,
            item_ids,
            all_dead,
            status,
        } => queues::handle_queue_retry(
            ctx,
            &project_root,
            &namespace,
            &queue_name,
            queues::RetryFilter {
                item_ids: &item_ids,
                all_dead,
                status_filter: status.as_deref(),
            },
        ),

        Request::QueueRetryBulk {
            project_root,
            namespace,
            queue_name,
            item_ids,
            all_dead,
            status_filter,
        } => queues::handle_queue_retry(
            ctx,
            &project_root,
            &namespace,
            &queue_name,
            queues::RetryFilter {
                item_ids: &item_ids,
                all_dead,
                status_filter: status_filter.as_deref(),
            },
        ),

        Request::QueueDrain {
            project_root,
            namespace,
            queue_name,
        } => queues::handle_queue_drain(ctx, &project_root, &namespace, &queue_name),

        Request::QueueFail {
            project_root,
            namespace,
            queue_name,
            item_id,
        } => queues::handle_queue_fail(ctx, &project_root, &namespace, &queue_name, &item_id),

        Request::QueueDone {
            project_root,
            namespace,
            queue_name,
            item_id,
        } => queues::handle_queue_done(ctx, &project_root, &namespace, &queue_name, &item_id),

        Request::QueuePrune {
            project_root,
            namespace,
            queue_name,
            all,
            dry_run,
        } => queues::handle_queue_prune(ctx, &project_root, &namespace, &queue_name, all, dry_run),

        Request::DecisionResolve {
            id,
            chosen,
            message,
        } => decisions::handle_decision_resolve(ctx, &id, chosen, message),

        Request::AgentResume {
            agent_id,
            kill,
            all,
        } => mutations::handle_agent_resume(ctx, agent_id, kill, all).await,

        Request::SessionPrune {
            all,
            dry_run,
            namespace,
        } => {
            let flags = mutations::PruneFlags {
                all,
                dry_run,
                namespace: namespace.as_deref(),
            };
            mutations::handle_session_prune(ctx, &flags).await
        }
    }
}

/// Load a runbook, falling back to the known project root for the namespace.
///
/// When the requested namespace differs from what would be resolved from `project_root`,
/// prefers the known project root for that namespace (supports `--project` from a different
/// directory). On total failure, calls `suggest_fn` to generate a "did you mean" hint.
fn load_runbook_with_fallback(
    project_root: &Path,
    namespace: &str,
    state: &Arc<Mutex<MaterializedState>>,
    load_fn: impl Fn(&Path) -> Result<oj_runbook::Runbook, String>,
    suggest_fn: impl FnOnce() -> String,
) -> Result<(oj_runbook::Runbook, PathBuf), Response> {
    // Check if the requested namespace differs from what project_root would resolve to.
    // This handles `--project <ns>` invoked from a different project directory.
    let project_namespace = oj_core::namespace::resolve_namespace(project_root);
    let known_root = {
        let st = state.lock();
        st.project_root_for_namespace(namespace)
    };

    // Determine the preferred root: use known root when namespace doesn't match project_root
    let (preferred_root, fallback_root) = if !namespace.is_empty() && namespace != project_namespace
    {
        // Namespace mismatch: prefer known root for the requested namespace
        match known_root.as_deref() {
            Some(known) => (known, Some(project_root)),
            None => (project_root, None),
        }
    } else {
        // Namespace matches or is empty: use project_root, fallback to known
        (project_root, known_root.as_deref())
    };

    match load_fn(preferred_root) {
        Ok(rb) => Ok((rb, preferred_root.to_path_buf())),
        Err(e) => {
            let alt_result = fallback_root
                .filter(|alt| *alt != preferred_root)
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

/// Check that a scoped resource exists in state, returning an error response if not.
///
/// The `check` closure receives the locked state and the scoped name, and should
/// return `true` if the resource exists. On failure, calls `suggest_fn` to
/// generate a "did you mean" hint.
fn require_scoped_resource(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    name: &str,
    resource_type: &str,
    check: impl FnOnce(&MaterializedState, &str) -> bool,
    suggest_fn: impl FnOnce() -> String,
) -> Result<(), Response> {
    let scoped = oj_core::scoped_name(namespace, name);
    let exists = check(&state.lock(), &scoped);
    if exists {
        Ok(())
    } else {
        let hint = suggest_fn();
        Err(Response::Error {
            message: format!("unknown {}: {}{}", resource_type, name, hint),
        })
    }
}

/// Check if a scoped resource exists in state.
fn scoped_exists(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    name: &str,
    check: impl FnOnce(&MaterializedState, &str) -> bool,
) -> bool {
    let scoped = oj_core::scoped_name(namespace, name);
    check(&state.lock(), &scoped)
}

/// Result of a batch start operation: (started_names, skipped_with_reasons).
type StartAllResult = (Vec<String>, Vec<(String, String)>);

/// Collect start results for a batch of resources.
///
/// Calls `start_fn` for each name and classifies results into started/skipped.
/// The `extract_name` closure extracts the started name from a success response.
fn collect_start_results(
    names: impl Iterator<Item = String>,
    start_fn: impl Fn(&str) -> Result<Response, ConnectionError>,
    extract_name: impl Fn(&Response) -> Option<String>,
) -> Result<StartAllResult, ConnectionError> {
    let mut started = Vec::new();
    let mut skipped = Vec::new();

    for name in names {
        match start_fn(&name) {
            Ok(ref resp) => {
                if let Some(started_name) = extract_name(resp) {
                    started.push(started_name);
                } else if let Response::Error { message } = resp {
                    skipped.push((name, message.clone()));
                } else {
                    skipped.push((name, "unexpected response".to_string()));
                }
            }
            Err(e) => {
                skipped.push((name, e.to_string()));
            }
        }
    }

    Ok((started, skipped))
}

#[cfg(test)]
fn make_listen_ctx(event_bus: crate::event_bus::EventBus, dir: &std::path::Path) -> ListenCtx {
    ListenCtx {
        event_bus,
        state: Arc::new(Mutex::new(MaterializedState::default())),
        orphans: Arc::new(Mutex::new(Vec::new())),
        metrics_health: Arc::new(Mutex::new(Default::default())),
        logs_path: dir.to_path_buf(),
        start_time: Instant::now(),
        shutdown: Arc::new(Notify::new()),
    }
}

#[cfg(test)]
pub(super) fn test_ctx(dir: &std::path::Path) -> ListenCtx {
    let wal = oj_storage::Wal::open(&dir.join("test.wal"), 0).unwrap();
    let (event_bus, _reader) = crate::event_bus::EventBus::new(wal);
    make_listen_ctx(event_bus, dir)
}

#[cfg(test)]
pub(super) fn test_ctx_with_wal(dir: &std::path::Path) -> (ListenCtx, Arc<Mutex<oj_storage::Wal>>) {
    let wal = oj_storage::Wal::open(&dir.join("test.wal"), 0).unwrap();
    let (event_bus, reader) = crate::event_bus::EventBus::new(wal);
    let wal = reader.wal();
    (make_listen_ctx(event_bus, dir), wal)
}

#[cfg(test)]
#[path = "../listener_tests.rs"]
mod tests;
