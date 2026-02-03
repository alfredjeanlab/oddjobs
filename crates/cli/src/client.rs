// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Daemon client for CLI commands

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::client_lifecycle::log_connection_error;
use crate::daemon_process::{
    cleanup_stale_socket, daemon_dir, daemon_socket, probe_socket, read_startup_error,
    start_daemon_background, stop_daemon_sync, wrap_with_startup_error,
};

use oj_core::AgentSignalKind;
use oj_daemon::protocol::{self, ProtocolError};
use oj_daemon::{Query, Request, Response};
use thiserror::Error;
use tokio::net::UnixStream;

// Timeout configuration (env vars in milliseconds)
fn parse_duration_ms(var: &str) -> Option<Duration> {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
}

/// Timeout for IPC requests (hello, status, event, query, shutdown)
pub fn timeout_ipc() -> Duration {
    parse_duration_ms("OJ_TIMEOUT_IPC_MS").unwrap_or(Duration::from_secs(5))
}

/// Timeout for waiting for daemon to start
pub fn timeout_connect() -> Duration {
    parse_duration_ms("OJ_TIMEOUT_CONNECT_MS").unwrap_or(Duration::from_secs(5))
}

/// Timeout for waiting for process to exit
pub fn timeout_exit() -> Duration {
    parse_duration_ms("OJ_TIMEOUT_EXIT_MS").unwrap_or(Duration::from_secs(2))
}

/// Polling interval for connection retries
pub fn poll_interval() -> Duration {
    parse_duration_ms("OJ_CONNECT_POLL_MS").unwrap_or(Duration::from_millis(50))
}

/// Client errors
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("Daemon not running")]
    DaemonNotRunning,

    #[error("Failed to start daemon: {0}")]
    DaemonStartFailed(String),

    #[error("Connection timeout waiting for daemon to start")]
    DaemonStartTimeout,

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Event rejected: {0}")]
    Rejected(String),

    #[error("Unexpected response from daemon")]
    UnexpectedResponse,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Could not determine state directory")]
    NoStateDir,
}

/// Result of a bulk cancel operation
pub struct CancelResult {
    pub cancelled: Vec<String>,
    pub already_terminal: Vec<String>,
    pub not_found: Vec<String>,
}

/// Daemon client
pub struct DaemonClient {
    socket_path: PathBuf,
}

impl DaemonClient {
    /// For action commands - auto-start with version check, max 1 restart per process
    ///
    /// Action commands mutate state and are user-initiated (run, resume, cancel, etc.).
    /// They should auto-start the daemon but limit restarts to prevent infinite loops.
    pub fn for_action() -> Result<Self, ClientError> {
        Self::connect_or_start_once()
    }

    /// For query commands - connect only, no restart
    ///
    /// Query commands read state and are user-initiated (status, list, show, logs, etc.).
    /// If the daemon is the wrong version, there's nothing useful to query anyway.
    pub fn for_query() -> Result<Self, ClientError> {
        Self::connect()
    }

    /// For signal commands - connect only, no restart
    ///
    /// Signal commands are operational, often agent-initiated (emit agent:signal).
    /// Restarting the daemon would lose the agent context, causing failures.
    /// This is a semantic alias for `for_query()` to document intent.
    pub fn for_signal() -> Result<Self, ClientError> {
        Self::connect()
    }

    /// Internal: connect_or_start with restart limit (max 1 restart per process)
    fn connect_or_start_once() -> Result<Self, ClientError> {
        static RESTARTED: AtomicBool = AtomicBool::new(false);

        // If we already restarted this process, don't do it again
        if RESTARTED.load(Ordering::SeqCst) {
            return Self::connect();
        }

        // Check version and restart if needed
        let daemon_dir = daemon_dir()?;
        let version_path = daemon_dir.join("daemon.version");
        if let Ok(daemon_version) = std::fs::read_to_string(&version_path) {
            let cli_version = concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH"));
            if daemon_version.trim() != cli_version {
                // Mark that we're restarting (before actually doing it)
                RESTARTED.store(true, Ordering::SeqCst);
                eprintln!(
                    "warn: daemon version {} does not match cli version {}, restarting daemon",
                    daemon_version.trim(),
                    cli_version
                );
                stop_daemon_sync();
            }
        }

        // Now connect or start
        match Self::connect() {
            Ok(client) => {
                if probe_socket(&client.socket_path) {
                    Ok(client)
                } else {
                    cleanup_stale_socket()?;
                    let child = start_daemon_background()?;
                    Self::connect_with_retry(timeout_connect(), child)
                }
            }
            Err(ClientError::DaemonNotRunning) => {
                let child = start_daemon_background()?;
                Self::connect_with_retry(timeout_connect(), child)
            }
            Err(e) => Err(wrap_with_startup_error(e)),
        }
    }

    /// Connect to daemon, auto-starting if not running
    pub fn connect_or_start() -> Result<Self, ClientError> {
        // Check version file before connecting - restart daemon if version mismatch
        let daemon_dir = daemon_dir()?;
        let version_path = daemon_dir.join("daemon.version");
        if let Ok(daemon_version) = std::fs::read_to_string(&version_path) {
            let cli_version = concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH"));
            if daemon_version.trim() != cli_version {
                // Version mismatch - stop old daemon first
                eprintln!(
                    "warn: daemon version {} does not match cli version {}, restarting daemon",
                    daemon_version.trim(),
                    cli_version
                );
                // Stop daemon synchronously (we're in a sync context inside a tokio runtime)
                stop_daemon_sync();
            }
        }

        match Self::connect() {
            Ok(client) => {
                // Verify the socket is actually accepting connections
                // (daemon may have crashed, leaving a stale socket file)
                if probe_socket(&client.socket_path) {
                    Ok(client)
                } else {
                    // Stale socket - clean up and start fresh
                    cleanup_stale_socket()?;
                    let child = start_daemon_background()?;
                    Self::connect_with_retry(timeout_connect(), child)
                }
            }
            Err(ClientError::DaemonNotRunning) => {
                // Start daemon in background
                let child = start_daemon_background()?;
                // Wait for socket with retry, watching for early exit
                Self::connect_with_retry(timeout_connect(), child)
            }
            Err(e) => Err(wrap_with_startup_error(e)),
        }
    }

    /// Connect to existing daemon (no auto-start)
    pub fn connect() -> Result<Self, ClientError> {
        let socket_path = daemon_socket()?;

        if !socket_path.exists() {
            let err = ClientError::DaemonNotRunning;
            log_connection_error(&err);
            return Err(err);
        }

        Ok(Self { socket_path })
    }

    fn connect_with_retry(
        timeout: Duration,
        mut child: std::process::Child,
    ) -> Result<Self, ClientError> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            // Check if daemon process exited early (startup failure)
            match child.try_wait() {
                Ok(Some(status)) => {
                    // Process exited - startup failed
                    // Poll for startup error in log (filesystem may need to sync)
                    let poll_start = Instant::now();
                    while poll_start.elapsed() < timeout_exit() {
                        if let Some(err) = read_startup_error() {
                            return Err(ClientError::DaemonStartFailed(err));
                        }
                        std::thread::sleep(poll_interval());
                    }
                    // No error found in log, return generic failure
                    return Err(ClientError::DaemonStartFailed(format!(
                        "exited with {}",
                        status
                    )));
                }
                Ok(None) => {
                    // Still running, try to connect
                }
                Err(_) => {
                    // Error checking status, assume still running
                }
            }

            match Self::connect() {
                Ok(client) => return Ok(client),
                Err(ClientError::DaemonNotRunning) => {
                    std::thread::sleep(poll_interval());
                }
                Err(e) => return Err(wrap_with_startup_error(e)),
            }
        }

        // Timeout - check log for startup errors
        Err(wrap_with_startup_error(ClientError::DaemonStartTimeout))
    }

    /// Send a request and receive a response with specific timeouts
    async fn send_with_timeout(
        &self,
        request: &Request,
        read_timeout: Duration,
        write_timeout: Duration,
    ) -> Result<Response, ClientError> {
        let stream = UnixStream::connect(&self.socket_path).await?;
        let (mut reader, mut writer) = stream.into_split();

        // Encode and send request with write timeout
        let data = protocol::encode(request)?;
        tokio::time::timeout(write_timeout, protocol::write_message(&mut writer, &data))
            .await
            .map_err(|_| ProtocolError::Timeout)??;

        // Read response with read timeout
        let response_bytes =
            tokio::time::timeout(read_timeout, protocol::read_message(&mut reader))
                .await
                .map_err(|_| ProtocolError::Timeout)??;

        let response: Response = protocol::decode(&response_bytes)?;
        Ok(response)
    }

    /// Send a request and receive a response
    pub async fn send(&self, request: &Request) -> Result<Response, ClientError> {
        match self
            .send_with_timeout(request, timeout_ipc(), timeout_ipc())
            .await
        {
            Ok(response) => Ok(response),
            Err(e) => {
                log_connection_error(&e);
                Err(e)
            }
        }
    }

    /// Emit an event to the daemon. If the connection fails (e.g., daemon
    /// socket is stale), reconnects and retries once with signal semantics
    /// (no daemon restart, as that would lose agent context).
    pub async fn emit_event(&self, event: oj_core::Event) -> Result<(), ClientError> {
        let request = Request::Event { event };
        match self.send_simple(&request).await {
            Ok(()) => Ok(()),
            Err(ClientError::Io(_)) | Err(ClientError::Protocol(_)) => {
                // Connection failed - try to reconnect with signal semantics (no restart)
                let new_client = DaemonClient::for_signal()?;
                new_client.send_simple(&request).await
            }
            Err(e) => Err(e),
        }
    }

    /// Helper for simple requests that expect Ok or Error responses
    async fn send_simple(&self, request: &Request) -> Result<(), ClientError> {
        match self.send(request).await? {
            Response::Ok => Ok(()),
            Response::Error { message } => Err(ClientError::Rejected(message)),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    /// Query for pipelines
    pub async fn list_pipelines(&self) -> Result<Vec<oj_daemon::PipelineSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListPipelines,
        };
        match self.send(&query).await? {
            Response::Pipelines { pipelines } => Ok(pipelines),
            other => Self::reject(other),
        }
    }

    /// Query for a specific pipeline
    pub async fn get_pipeline(
        &self,
        id: &str,
    ) -> Result<Option<oj_daemon::PipelineDetail>, ClientError> {
        let request = Request::Query {
            query: Query::GetPipeline { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Pipeline { pipeline } => Ok(pipeline.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Get daemon status
    pub async fn status(&self) -> Result<(u64, usize, usize, usize), ClientError> {
        match self.send(&Request::Status).await? {
            Response::Status {
                uptime_secs,
                pipelines_active,
                sessions_active,
                orphan_count,
            } => Ok((uptime_secs, pipelines_active, sessions_active, orphan_count)),
            other => Self::reject(other),
        }
    }

    /// Request daemon shutdown
    pub async fn shutdown(&self, kill: bool) -> Result<(), ClientError> {
        match self.send(&Request::Shutdown { kill }).await? {
            Response::Ok | Response::ShuttingDown => Ok(()),
            other => Self::reject(other),
        }
    }

    /// Get daemon version via Hello handshake
    pub async fn hello(&self) -> Result<String, ClientError> {
        let request = Request::Hello {
            version: concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH")).to_string(),
        };
        match self.send(&request).await? {
            Response::Hello { version } => Ok(version),
            other => Self::reject(other),
        }
    }

    /// Query for agents across all pipelines
    pub async fn list_agents(
        &self,
        pipeline_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<oj_daemon::AgentSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListAgents {
                pipeline_id: pipeline_id.map(|s| s.to_string()),
                status: status.map(|s| s.to_string()),
            },
        };
        match self.send(&query).await? {
            Response::Agents { agents } => Ok(agents),
            other => Self::reject(other),
        }
    }

    /// Query for sessions
    pub async fn list_sessions(&self) -> Result<Vec<oj_daemon::SessionSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListSessions,
        };
        match self.send(&query).await? {
            Response::Sessions { sessions } => Ok(sessions),
            other => Self::reject(other),
        }
    }

    /// Send a message to a running agent
    pub async fn agent_send(&self, agent_id: &str, message: &str) -> Result<(), ClientError> {
        let request = Request::AgentSend {
            agent_id: agent_id.to_string(),
            message: message.to_string(),
        };
        self.send_simple(&request).await
    }

    /// Send input to a session
    pub async fn session_send(&self, id: &str, input: &str) -> Result<(), ClientError> {
        let request = Request::SessionSend {
            id: id.to_string(),
            input: input.to_string(),
        };
        self.send_simple(&request).await
    }

    /// Resume monitoring for an escalated pipeline
    pub async fn pipeline_resume(
        &self,
        id: &str,
        message: Option<&str>,
        vars: &HashMap<String, String>,
    ) -> Result<(), ClientError> {
        let request = Request::PipelineResume {
            id: id.to_string(),
            message: message.map(String::from),
            vars: vars.clone(),
        };
        self.send_simple(&request).await
    }

    /// Cancel one or more pipelines by ID
    pub async fn pipeline_cancel(&self, ids: &[String]) -> Result<CancelResult, ClientError> {
        let request = Request::PipelineCancel { ids: ids.to_vec() };
        match self.send(&request).await? {
            Response::PipelinesCancelled {
                cancelled,
                already_terminal,
                not_found,
            } => Ok(CancelResult {
                cancelled,
                already_terminal,
                not_found,
            }),
            other => Self::reject(other),
        }
    }

    /// Query for workspaces
    pub async fn list_workspaces(&self) -> Result<Vec<oj_daemon::WorkspaceSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListWorkspaces,
        };
        match self.send(&query).await? {
            Response::Workspaces { workspaces } => Ok(workspaces),
            Response::Error { message } => Err(ClientError::Rejected(message)),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    /// Query for a specific workspace
    pub async fn get_workspace(
        &self,
        id: &str,
    ) -> Result<Option<oj_daemon::WorkspaceDetail>, ClientError> {
        let request = Request::Query {
            query: Query::GetWorkspace { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Workspace { workspace } => Ok(workspace.map(|b| *b)),
            Response::Error { message } => Err(ClientError::Rejected(message)),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    /// Peek at a session's tmux pane output
    pub async fn peek_session(
        &self,
        session_id: &str,
        with_color: bool,
    ) -> Result<String, ClientError> {
        let request = Request::PeekSession {
            session_id: session_id.to_string(),
            with_color,
        };
        match self.send(&request).await? {
            Response::SessionPeek { output } => Ok(output),
            other => Self::reject(other),
        }
    }

    /// Get pipeline logs
    pub async fn get_pipeline_logs(
        &self,
        id: &str,
        lines: usize,
    ) -> Result<(PathBuf, String), ClientError> {
        let request = Request::Query {
            query: Query::GetPipelineLogs {
                id: id.to_string(),
                lines,
            },
        };
        match self.send(&request).await? {
            Response::PipelineLogs { log_path, content } => Ok((log_path, content)),
            other => Self::reject(other),
        }
    }

    /// Get agent logs
    pub async fn get_agent_logs(
        &self,
        id: &str,
        step: Option<&str>,
        lines: usize,
    ) -> Result<(PathBuf, String, Vec<String>), ClientError> {
        let request = Request::Query {
            query: Query::GetAgentLogs {
                id: id.to_string(),
                step: step.map(|s| s.to_string()),
                lines,
            },
        };
        match self.send(&request).await? {
            Response::AgentLogs {
                log_path,
                content,
                steps,
            } => Ok((log_path, content, steps)),
            other => Self::reject(other),
        }
    }

    /// Run a command from the project runbook
    pub async fn run_command(
        &self,
        project_root: &Path,
        invoke_dir: &Path,
        namespace: &str,
        command: &str,
        args: &[String],
        named_args: &HashMap<String, String>,
    ) -> Result<(String, String), ClientError> {
        let request = Request::RunCommand {
            project_root: project_root.to_path_buf(),
            invoke_dir: invoke_dir.to_path_buf(),
            namespace: namespace.to_string(),
            command: command.to_string(),
            args: args.to_vec(),
            named_args: named_args.clone(),
        };
        match self.send(&request).await? {
            Response::CommandStarted {
                pipeline_id,
                pipeline_name,
            } => Ok((pipeline_id, pipeline_name)),
            other => Self::reject(other),
        }
    }

    /// Delete a specific workspace by ID
    pub async fn workspace_drop(
        &self,
        id: &str,
    ) -> Result<Vec<oj_daemon::WorkspaceEntry>, ClientError> {
        self.send_workspace_drop(Request::WorkspaceDrop { id: id.to_string() })
            .await
    }

    /// Delete all failed workspaces
    pub async fn workspace_drop_failed(
        &self,
    ) -> Result<Vec<oj_daemon::WorkspaceEntry>, ClientError> {
        self.send_workspace_drop(Request::WorkspaceDropFailed).await
    }

    /// Delete all workspaces
    pub async fn workspace_drop_all(&self) -> Result<Vec<oj_daemon::WorkspaceEntry>, ClientError> {
        self.send_workspace_drop(Request::WorkspaceDropAll).await
    }

    async fn send_workspace_drop(
        &self,
        request: Request,
    ) -> Result<Vec<oj_daemon::WorkspaceEntry>, ClientError> {
        match self.send(&request).await? {
            Response::WorkspacesDropped { dropped } => Ok(dropped),
            other => Self::reject(other),
        }
    }

    /// Prune old terminal pipelines and their log files
    pub async fn pipeline_prune(
        &self,
        all: bool,
        failed: bool,
        dry_run: bool,
    ) -> Result<(Vec<oj_daemon::PipelineEntry>, usize), ClientError> {
        let req = Request::PipelinePrune {
            all,
            failed,
            dry_run,
        };
        match self.send(&req).await? {
            Response::PipelinesPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    /// Prune agent logs from terminal pipelines
    pub async fn agent_prune(
        &self,
        all: bool,
        dry_run: bool,
    ) -> Result<(Vec<oj_daemon::AgentEntry>, usize), ClientError> {
        match self.send(&Request::AgentPrune { all, dry_run }).await? {
            Response::AgentsPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    /// Prune old workspaces from terminal pipelines
    pub async fn workspace_prune(
        &self,
        all: bool,
        dry_run: bool,
    ) -> Result<(Vec<oj_daemon::WorkspaceEntry>, usize), ClientError> {
        match self.send(&Request::WorkspacePrune { all, dry_run }).await? {
            Response::WorkspacesPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    /// Prune stopped workers from daemon state
    pub async fn worker_prune(
        &self,
        all: bool,
        dry_run: bool,
    ) -> Result<(Vec<oj_daemon::WorkerEntry>, usize), ClientError> {
        match self.send(&Request::WorkerPrune { all, dry_run }).await? {
            Response::WorkersPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    fn reject<T>(resp: Response) -> Result<T, ClientError> {
        match resp {
            Response::Error { message } => Err(ClientError::Rejected(message)),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    /// Get cross-project status overview
    pub async fn status_overview(
        &self,
    ) -> Result<(u64, Vec<oj_daemon::NamespaceStatus>), ClientError> {
        let query = Request::Query {
            query: Query::StatusOverview,
        };
        match self.send(&query).await? {
            Response::StatusOverview {
                uptime_secs,
                namespaces,
            } => Ok((uptime_secs, namespaces)),
            other => Self::reject(other),
        }
    }

    /// Query if an agent has signaled completion (for stop hook)
    pub async fn query_agent_signal(
        &self,
        agent_id: &str,
    ) -> Result<AgentSignalResponse, ClientError> {
        let request = Request::Query {
            query: Query::GetAgentSignal {
                agent_id: agent_id.to_string(),
            },
        };
        match self.send(&request).await? {
            Response::AgentSignal {
                signaled,
                kind,
                message,
            } => Ok(AgentSignalResponse {
                signaled,
                kind,
                message,
            }),
            other => Self::reject(other),
        }
    }

    /// List orphaned pipelines detected at startup
    pub async fn list_orphans(&self) -> Result<Vec<oj_daemon::OrphanSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListOrphans,
        };
        match self.send(&request).await? {
            Response::Orphans { orphans } => Ok(orphans),
            other => Self::reject(other),
        }
    }

    /// Dismiss an orphaned pipeline by deleting its breadcrumb
    pub async fn dismiss_orphan(&self, id: &str) -> Result<(), ClientError> {
        let request = Request::Query {
            query: Query::DismissOrphan { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Ok => Ok(()),
            other => Self::reject(other),
        }
    }
}

/// Response from agent signal query
pub struct AgentSignalResponse {
    /// Whether the agent has signaled completion
    pub signaled: bool,
    // NOTE(compat): Exposed for future CLI display of signal details
    #[allow(dead_code)]
    pub kind: Option<AgentSignalKind>,
    // NOTE(compat): same as above
    #[allow(dead_code)]
    pub message: Option<String>,
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
