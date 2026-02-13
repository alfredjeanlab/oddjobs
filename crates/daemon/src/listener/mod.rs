// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Listener task for handling socket I/O.
//!
//! The Listener runs in a spawned task, accepting connections and
//! handling them without blocking the engine loop. Events are emitted
//! onto the EventBus for processing by the engine.

mod attach;
mod commands;
mod coop;
mod crons;
mod decisions;
mod lifecycle;
mod mutations;
mod query;
mod queues;
mod suggest;
mod workers;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Instant;

use crate::storage::MaterializedState;
use oj_core::Event;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::adapters::AgentAdapter;
use crate::event_bus::EventBus;
use oj_core::{Breadcrumb, MetricsHealth};

use crate::env::{ipc_timeout, PROTOCOL_VERSION};
use crate::protocol::{self, Request, Response};

/// Shared daemon context for all request handlers.
pub(crate) struct ListenCtx {
    pub event_bus: EventBus,
    pub state: Arc<Mutex<MaterializedState>>,
    pub orphans: Arc<Mutex<Vec<Breadcrumb>>>,
    pub metrics_health: Arc<Mutex<MetricsHealth>>,
    pub state_dir: PathBuf,
    pub logs_path: PathBuf,
    pub start_time: Instant,
    pub shutdown: Arc<Notify>,
    /// Auth token for TCP connections (from `OJ_AUTH_TOKEN`).
    /// When set, TCP clients must provide this token in the Hello handshake.
    pub auth_token: Option<String>,
    /// Agent adapter for infrastructure (attach proxying via get_coop_host)
    pub agent: Arc<dyn AgentAdapter>,
}

/// Listener task for accepting socket connections.
pub(crate) struct Listener {
    unix: UnixListener,
    tcp: Option<TcpListener>,
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
    /// Create a new listener with Unix socket only.
    pub fn new(unix: UnixListener, ctx: Arc<ListenCtx>) -> Self {
        Self { unix, tcp: None, ctx }
    }

    /// Create a new listener with both Unix socket and TCP.
    pub fn with_tcp(unix: UnixListener, tcp: TcpListener, ctx: Arc<ListenCtx>) -> Self {
        Self { unix, tcp: Some(tcp), ctx }
    }

    /// Run the listener loop until shutdown, spawning tasks for each connection.
    pub async fn run(mut self) {
        match self.tcp.take() {
            Some(tcp) => self.run_dual(tcp).await,
            None => self.run_unix_only().await,
        }
    }

    /// Listen on Unix socket only (existing behavior).
    async fn run_unix_only(self) {
        loop {
            match self.unix.accept().await {
                Ok((stream, _)) => {
                    let ctx = Arc::clone(&self.ctx);
                    tokio::spawn(async move {
                        let (reader, writer) = stream.into_split();
                        if let Err(e) =
                            handle_connection(reader, writer, ConnectionSource::Unix, &ctx).await
                        {
                            log_connection_error(e);
                        }
                    });
                }
                Err(e) => error!("Unix accept error: {}", e),
            }
        }
    }

    /// Listen on both Unix socket and TCP.
    async fn run_dual(self, tcp: TcpListener) {
        loop {
            tokio::select! {
                result = self.unix.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            let ctx = Arc::clone(&self.ctx);
                            tokio::spawn(async move {
                                let (reader, writer) = stream.into_split();
                                if let Err(e) = handle_connection(reader, writer, ConnectionSource::Unix, &ctx).await {
                                    log_connection_error(e);
                                }
                            });
                        }
                        Err(e) => error!("Unix accept error: {}", e),
                    }
                }
                result = tcp.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            debug!("TCP connection from {}", addr);
                            let ctx = Arc::clone(&self.ctx);
                            tokio::spawn(async move {
                                let (reader, writer) = stream.into_split();
                                if let Err(e) = handle_connection(reader, writer, ConnectionSource::Tcp, &ctx).await {
                                    log_connection_error(e);
                                }
                            });
                        }
                        Err(e) => error!("TCP accept error: {}", e),
                    }
                }
            }
        }
    }
}

/// Source of a connection (for auth decisions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionSource {
    /// Local Unix socket — trusted, no auth required.
    Unix,
    /// Remote TCP — requires auth token in Hello handshake.
    Tcp,
}

fn log_connection_error(e: ConnectionError) {
    match e {
        ConnectionError::Protocol(protocol::ProtocolError::ConnectionClosed) => {
            debug!("Client disconnected")
        }
        ConnectionError::Protocol(protocol::ProtocolError::Timeout) => {
            warn!("Connection timeout")
        }
        _ => error!("Connection error: {}", e),
    }
}

/// Handle a single client connection.
///
/// Creates a `CancellationToken` and races the request handler against client
/// disconnect detection. If the client closes the connection before the handler
/// completes (e.g., CLI timeout), the token is cancelled and the handler is
/// dropped, preventing wasted work from orphaned handler tasks.
///
/// Generic over reader/writer types so it works with both Unix and TCP streams.
async fn handle_connection<R, W>(
    mut reader: R,
    mut writer: W,
    source: ConnectionSource,
    ctx: &ListenCtx,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + AsyncReadExt + Unpin + Send + 'static,
    W: AsyncWrite + AsyncWriteExt + Unpin + Send + 'static,
{
    // Read request with timeout
    let request = protocol::read_request(&mut reader, ipc_timeout()).await?;

    // TCP connections must authenticate via Hello handshake as the first request
    if source == ConnectionSource::Tcp {
        if let Request::Hello { ref token, .. } = request {
            if let Some(ref expected) = ctx.auth_token {
                match token {
                    Some(provided) if provided == expected => {
                        // Token matches — proceed normally
                    }
                    _ => {
                        let response = Response::Error { message: "unauthorized".to_string() };
                        let _ =
                            protocol::write_response(&mut writer, &response, ipc_timeout()).await;
                        return Ok(());
                    }
                }
            }
            // No auth_token configured on daemon — allow all TCP connections
        } else {
            // TCP connections must start with Hello
            let response =
                Response::Error { message: "TCP connections must start with Hello".to_string() };
            let _ = protocol::write_response(&mut writer, &response, ipc_timeout()).await;
            return Ok(());
        }
    }

    // Log queries at debug level (frequent polling), other requests at info
    if matches!(request, Request::Query { .. }) {
        debug!(request = ?request, "received query");
    } else {
        info!(request = ?request, "received request");
    }

    // AgentAttach is a connection-upgrading request — after the handshake,
    // the connection becomes a bidirectional byte stream. Handle it before
    // the normal request/response dispatch.
    if let Request::AgentAttach { ref id, ref token } = request {
        return attach::handle_agent_attach(id, token.as_deref(), reader, writer, ctx).await;
    }

    // Race handler against client disconnect
    let token = CancellationToken::new();
    let response = tokio::select! {
        result = handle_request(request, ctx, token.clone()) => result?,
        _ = detect_client_disconnect(&mut reader) => {
            token.cancel();
            debug!("Client disconnected, cancelling handler");
            return Ok(());
        }
    };

    debug!("Sending response: {:?}", response);

    // Write response with timeout
    protocol::write_response(&mut writer, &response, ipc_timeout()).await?;

    Ok(())
}

/// Detect client disconnect by reading from the socket after the request.
///
/// In the request-response protocol, the client sends one request then waits.
/// If the client disconnects (e.g., CLI timeout), reading returns 0 bytes (EOF).
async fn detect_client_disconnect<R: AsyncReadExt + Unpin>(reader: &mut R) {
    let mut buf = [0u8; 1];
    let _ = reader.read(&mut buf).await;
}

/// Handle a single request and return a response.
///
/// The `cancel` token is checked by subprocess-calling handlers between iterations
/// to exit early when the client has disconnected.
async fn handle_request(
    request: Request,
    ctx: &ListenCtx,
    cancel: CancellationToken,
) -> Result<Response, ConnectionError> {
    match request {
        Request::Ping => Ok(Response::Pong),

        Request::Hello { version: _, token: _ } => {
            // Auth token validation for TCP connections happens at the transport
            // layer before dispatching to handle_request. By the time we get here,
            // the connection is already authenticated.
            Ok(Response::Hello { version: PROTOCOL_VERSION.to_string() })
        }

        Request::Event { event } => {
            mutations::emit(&ctx.event_bus, event)?;
            Ok(Response::Ok)
        }

        Request::Query { query } => Ok(query::handle_query(ctx, query)),

        Request::Shutdown { kill } => {
            if kill {
                coop::kill_state_agents(&ctx.state, &ctx.state_dir).await;
            }
            ctx.shutdown.notify_one();
            Ok(Response::ShuttingDown)
        }

        Request::Status => Ok(mutations::handle_status(ctx)),

        Request::AgentSend { id, message } => mutations::handle_agent_send(ctx, id, message).await,

        Request::AgentKill { id } => mutations::handle_agent_kill(ctx, id).await,

        Request::JobResume { id, message, vars, kill, all } => {
            if all {
                mutations::handle_job_resume_all(ctx, kill)
            } else {
                mutations::handle_job_resume(ctx, id, message, vars, kill)
            }
        }

        Request::JobResumeAll { kill } => mutations::handle_job_resume_all(ctx, kill),

        Request::JobCancel { ids } => mutations::handle_job_cancel(ctx, ids),

        Request::JobSuspend { ids } => mutations::handle_job_suspend(ctx, ids),

        Request::RunCommand { project_path, invoke_dir, project, command, args, kwargs } => {
            commands::handle_run_command(commands::RunCommandParams {
                project_path: &project_path,
                invoke_dir: &invoke_dir,
                project: &project,
                command: &command,
                args: &args,
                named_args: &kwargs,
                ctx,
            })
            .await
        }

        Request::WorkspaceDrop { id } => {
            mutations::handle_workspace_drop(ctx, Some(&id), false, false).await
        }

        Request::WorkspaceDropFailed => {
            mutations::handle_workspace_drop(ctx, None, true, false).await
        }

        Request::WorkspaceDropAll => mutations::handle_workspace_drop(ctx, None, false, true).await,

        Request::JobPrune { all, failed, orphans: prune_orphans, dry_run, project } => {
            let flags = mutations::PruneFlags { all, dry_run, project: project.as_deref() };
            mutations::handle_job_prune(ctx, &flags, failed, prune_orphans)
        }

        Request::AgentPrune { all, dry_run } => {
            let flags = mutations::PruneFlags { all, dry_run, project: None };
            mutations::handle_agent_prune(ctx, &flags)
        }

        Request::WorkspacePrune { all, dry_run, project } => {
            let flags = mutations::PruneFlags { all, dry_run, project: project.as_deref() };
            mutations::handle_workspace_prune(ctx, &flags, &cancel).await
        }

        Request::WorkerPrune { all, dry_run, project } => {
            let flags = mutations::PruneFlags { all, dry_run, project: project.as_deref() };
            workers::handle_worker_prune(ctx, &flags)
        }

        Request::CronPrune { all, dry_run } => {
            let flags = mutations::PruneFlags { all, dry_run, project: None };
            crons::handle_cron_prune(ctx, &flags)
        }

        Request::WorkerStart { project_path, project, worker, all } => {
            workers::handle_worker_start(ctx, &project_path, &project, &worker, all)
        }

        Request::WorkerWake { worker, project } => {
            mutations::emit(&ctx.event_bus, Event::WorkerWake { worker, project })?;
            Ok(Response::Ok)
        }

        Request::WorkerStop { worker, project, project_path, all } => {
            workers::handle_worker_stop(ctx, &worker, &project, project_path.as_deref(), all)
        }

        Request::WorkerRestart { project_path, project, worker } => {
            workers::handle_worker_restart(ctx, &project_path, &project, &worker)
        }

        Request::WorkerResize { worker, project, concurrency } => {
            workers::handle_worker_resize(ctx, &worker, &project, concurrency)
        }

        Request::CronStart { project_path, project, cron, all } => {
            crons::handle_cron_start(ctx, &project_path, &project, &cron, all)
        }

        Request::CronStop { cron, project, project_path, all } => {
            crons::handle_cron_stop(ctx, &cron, &project, project_path.as_deref(), all)
        }

        Request::CronRestart { project_path, project, cron } => {
            crons::handle_cron_restart(ctx, &project_path, &project, &cron)
        }

        Request::CronOnce { project_path, project, cron } => {
            crons::handle_cron_once(ctx, &project_path, &project, &cron).await
        }

        Request::QueuePush { project_path, project, queue, data } => {
            queues::handle_queue_push(ctx, &project_path, &project, &queue, data)
        }

        Request::QueueDrop { project_path, project, queue, item_id } => {
            queues::handle_queue_drop(ctx, &project_path, &project, &queue, &item_id)
        }

        Request::QueueRetry { project_path, project, queue, item_ids, all_dead, status } => {
            queues::handle_queue_retry(
                ctx,
                &project_path,
                &project,
                &queue,
                queues::RetryFilter {
                    item_ids: &item_ids,
                    all_dead,
                    status_filter: status.as_deref(),
                },
            )
        }

        Request::QueueDrain { project_path, project, queue } => {
            queues::handle_queue_drain(ctx, &project_path, &project, &queue)
        }

        Request::QueueFail { project_path, project, queue, item_id } => {
            queues::handle_queue_fail(ctx, &project_path, &project, &queue, &item_id)
        }

        Request::QueueDone { project_path, project, queue, item_id } => {
            queues::handle_queue_done(ctx, &project_path, &project, &queue, &item_id)
        }

        Request::QueuePrune { project_path, project, queue, all, dry_run } => {
            queues::handle_queue_prune(ctx, &project_path, &project, &queue, all, dry_run)
        }

        Request::DecisionResolve { id, choices, message } => {
            decisions::handle_decision_resolve(ctx, id.as_str(), choices, message)
        }

        Request::AgentResume { id, kill, all } => {
            mutations::handle_agent_resume(ctx, id, kill, all, &cancel).await
        }

        // Intercepted in handle_connection before reaching handle_request
        Request::AgentAttach { .. } => unreachable!(),
    }
}

/// Load a runbook, falling back to the known project root for the project.
///
/// When the requested project differs from what would be resolved from `project_path`,
/// prefers the known project root for that project (supports `--project` from a different
/// directory). On total failure, calls `suggest_fn` to generate a "did you mean" hint.
fn load_runbook_with_fallback(
    project_path: &Path,
    project: &str,
    state: &Arc<Mutex<MaterializedState>>,
    load_fn: impl Fn(&Path) -> Result<oj_runbook::Runbook, String>,
    suggest_fn: impl FnOnce() -> String,
) -> Result<(oj_runbook::Runbook, PathBuf), Response> {
    // Check if the requested project differs from what project_path would resolve to.
    // This handles `--project <ns>` invoked from a different project directory.
    let project_namespace = oj_core::project::resolve_namespace(project_path);
    let known_root = {
        let st = state.lock();
        st.project_path_for_namespace(project)
    };

    // Determine the preferred root: use known root when project doesn't match project_path
    let (preferred_root, fallback_root) = if !project.is_empty() && project != project_namespace {
        // Namespace mismatch: prefer known root for the requested project
        match known_root.as_deref() {
            Some(known) => (known, Some(project_path)),
            None => (project_path, None),
        }
    } else {
        // Namespace matches or is empty: use project_path, fallback to known
        (project_path, known_root.as_deref())
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
                    Err(Response::Error { message: format!("{}{}", e, hint) })
                }
            }
        }
    }
}

/// Resolve the effective project path, preferring the known path for a cross-project request.
fn resolve_effective_project_path(
    project_path: &Path,
    project: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> PathBuf {
    if !project.is_empty() && project != oj_core::project::resolve_namespace(project_path) {
        if let Some(known) = state.lock().project_path_for_namespace(project) {
            return known;
        }
    }
    project_path.to_path_buf()
}

#[cfg(test)]
fn make_listen_ctx(event_bus: crate::event_bus::EventBus, dir: &std::path::Path) -> ListenCtx {
    ListenCtx {
        event_bus,
        state: Arc::new(Mutex::new(MaterializedState::default())),
        orphans: Arc::new(Mutex::new(Vec::new())),
        metrics_health: Arc::new(Mutex::new(Default::default())),
        state_dir: dir.to_path_buf(),
        logs_path: dir.to_path_buf(),
        start_time: Instant::now(),
        shutdown: Arc::new(Notify::new()),
        auth_token: None,
        agent: std::sync::Arc::new(crate::adapters::FakeAgentAdapter::new()),
    }
}

#[cfg(test)]
pub(super) fn test_ctx(dir: &std::path::Path) -> ListenCtx {
    let wal = crate::storage::Wal::open(&dir.join("test.wal"), 0).unwrap();
    let (event_bus, _reader) = crate::event_bus::EventBus::new(wal);
    make_listen_ctx(event_bus, dir)
}

#[cfg(test)]
pub(super) fn test_ctx_with_wal(
    dir: &std::path::Path,
) -> (ListenCtx, Arc<Mutex<crate::storage::Wal>>) {
    let wal = crate::storage::Wal::open(&dir.join("test.wal"), 0).unwrap();
    let (event_bus, reader) = crate::event_bus::EventBus::new(wal);
    let wal = Arc::clone(&reader.wal);
    (make_listen_ctx(event_bus, dir), wal)
}

#[cfg(test)]
mod test_fixtures;

#[cfg(test)]
#[path = "../listener_tests.rs"]
mod tests;
