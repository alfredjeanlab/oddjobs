// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Daemon lifecycle and diagnostic logging for the CLI client.

use std::path::PathBuf;

use crate::client::{timeout_exit, ClientError, DaemonClient};
use crate::daemon_process::{
    cleanup_stale_pid, daemon_dir, daemon_socket, force_kill_daemon, process_exists,
    read_daemon_pid, wait_for_exit,
};

/// Stop the daemon (graceful first, then forceful)
/// Returns true if daemon was stopped, false if it wasn't running
pub async fn daemon_stop(kill: bool) -> Result<bool, ClientError> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(ClientError::DaemonNotRunning) => {
            // Clean up any stale files
            if let Ok(dir) = daemon_dir() {
                cleanup_stale_pid(&dir);
            }
            return Ok(false);
        }
        Err(e) => return Err(e),
    };

    // Try graceful shutdown (timeout handled by send())
    let shutdown_result = client.shutdown(kill).await;

    if let Some(pid) = read_daemon_pid()? {
        if shutdown_result.is_ok() {
            // Graceful shutdown succeeded, wait for process to exit
            wait_for_exit(pid, timeout_exit()).await;
        }

        // Force kill if still running
        if process_exists(pid) {
            force_kill_daemon(pid);
            wait_for_exit(pid, timeout_exit()).await;
        }
    }

    // Clean up stale files
    if let Ok(dir) = daemon_dir() {
        cleanup_stale_pid(&dir);
    }

    Ok(true)
}

/// Write a diagnostic message to `~/.local/state/oj/cli.log`.
fn write_cli_log(message: String) {
    use std::io::Write;
    use std::time::SystemTime;

    let log_path = daemon_dir()
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".local/state/oj"))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
        .join("cli.log");

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let pid = std::process::id();
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "(unknown)".to_string());
        let state_dir = std::env::var("OJ_STATE_DIR").unwrap_or_else(|_| "(not set)".to_string());

        let _ = writeln!(
            file,
            "[ts={}] pid={} cwd={} OJ_STATE_DIR={} {}",
            timestamp, pid, cwd, state_dir, message
        );
    }
}

/// Log a connection error for debugging.
///
/// Writes diagnostic info to `~/.local/state/oj/cli.log` when the CLI
/// fails to connect to the daemon. This helps debug issues in spawned agents
/// where stdout/stderr may not be visible.
pub fn log_connection_error(error: &ClientError) {
    let socket_path = daemon_socket()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".to_string());
    write_cli_log(format!("socket={} error={}", socket_path, error));
}
