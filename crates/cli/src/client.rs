// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Daemon client for CLI commands

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::client_lifecycle::log_connection_error;
use crate::color;
use crate::daemon_process::{
    cleanup_stale_socket, daemon_dir, daemon_socket, probe_socket, read_startup_error,
    start_daemon_background, stop_daemon_sync, wrap_with_startup_error,
};

use oj_daemon::protocol::{self, ProtocolError};
use oj_daemon::{Request, Response};
use thiserror::Error;
use tokio::net::UnixStream;

#[path = "client_queries.rs"]
mod queries;
pub use queries::RunCommandResult;

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

/// Polling interval for `oj pipeline wait` / `oj agent wait`
pub fn wait_poll_interval() -> Duration {
    parse_duration_ms("OJ_WAIT_POLL_MS").unwrap_or(Duration::from_secs(1))
}

/// How long `oj run` waits for a pipeline to start before returning
pub fn run_wait_timeout() -> Duration {
    parse_duration_ms("OJ_RUN_WAIT_MS").unwrap_or(Duration::from_secs(10))
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
                    "{} daemon version {} does not match cli version {}, restarting daemon",
                    color::yellow("warn:"),
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
                    "{} daemon version {} does not match cli version {}, restarting daemon",
                    color::yellow("warn:"),
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
    pub(crate) async fn send_simple(&self, request: &Request) -> Result<(), ClientError> {
        match self.send(request).await? {
            Response::Ok => Ok(()),
            Response::Error { message } => Err(ClientError::Rejected(message)),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }

    pub(crate) fn reject<T>(resp: Response) -> Result<T, ClientError> {
        match resp {
            Response::Error { message } => Err(ClientError::Rejected(message)),
            _ => Err(ClientError::UnexpectedResponse),
        }
    }
}

/// Response from agent signal query
pub struct AgentSignalResponse {
    pub signaled: bool,
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
