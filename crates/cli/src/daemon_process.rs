// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Daemon process management utilities.
//!
//! Functions for starting, stopping, and monitoring the oj daemon process.

use crate::client::ClientError;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Start the daemon in the background, returning the child process handle
pub fn start_daemon_background() -> Result<std::process::Child, ClientError> {
    let ojd_path = find_ojd_binary()?;

    Command::new(&ojd_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| ClientError::DaemonStartFailed(e.to_string()))
}

/// Stop the daemon synchronously using SIGTERM + polling.
///
/// Used during version-mismatch restart where we're in a sync context
/// inside a tokio runtime (can't use block_on).
pub fn stop_daemon_sync() {
    if let Ok(Some(pid)) = read_daemon_pid() {
        kill_signal("-15", pid);

        let start = Instant::now();
        let timeout = super::client::timeout_exit();
        while start.elapsed() < timeout {
            if !process_exists(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        if process_exists(pid) {
            force_kill_daemon(pid);
            let start = Instant::now();
            while start.elapsed() < timeout {
                if !process_exists(pid) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    if let Ok(dir) = daemon_dir() {
        cleanup_stale_pid(&dir);
    }
}

/// Wait for a process to exit
pub async fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !process_exists(pid) {
            return true;
        }
        tokio::time::sleep(super::client::poll_interval()).await;
    }
    false
}

/// Find the ojd binary
fn find_ojd_binary() -> Result<PathBuf, ClientError> {
    if let Some(path) = crate::env::daemon_binary() {
        return Ok(PathBuf::from(path));
    }

    let current_exe = std::env::current_exe().ok();

    // Only use CARGO_MANIFEST_DIR if the CLI itself is a debug build.
    // This prevents version mismatches when agents run in tmux sessions that
    // inherit CARGO_MANIFEST_DIR from a dev environment but use release builds.
    let is_debug_build = current_exe
        .as_ref()
        .and_then(|p| p.to_str())
        .map(|s| s.contains("target/debug"))
        .unwrap_or(false);

    if is_debug_build {
        if let Some(manifest_dir) = crate::env::cargo_manifest_dir() {
            let dev_path = PathBuf::from(manifest_dir)
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("target/debug/ojd"));
            if let Some(path) = dev_path {
                if path.exists() {
                    return Ok(path);
                }
            }
        }
    }

    if let Some(ref exe) = current_exe {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("ojd");
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }

    Ok(PathBuf::from("ojd"))
}

/// Get the socket path for the user-level daemon.
pub fn daemon_socket() -> Result<PathBuf, ClientError> {
    let dir = daemon_dir()?;
    Ok(dir.join("daemon.sock"))
}

/// Get the state directory for oj (user-level daemon).
pub fn daemon_dir() -> Result<PathBuf, ClientError> {
    crate::env::state_dir()
}

/// Tmux socket directory used by the daemon for session isolation.
pub fn daemon_tmux_dir() -> Result<PathBuf, ClientError> {
    Ok(daemon_dir()?.join("tmux"))
}

/// Clean up orphaned PID file during shutdown.
pub fn cleanup_stale_pid(dir: &std::path::Path) {
    let pid_path = dir.join("daemon.pid");
    if pid_path.exists() {
        let _ = std::fs::remove_file(&pid_path);
    }
}

/// Get the PID from the daemon PID file, if it exists
pub fn read_daemon_pid() -> Result<Option<u32>, ClientError> {
    let dir = daemon_dir()?;
    let pid_path = dir.join("daemon.pid");

    if !pid_path.exists() {
        return Ok(None);
    }

    match std::fs::read_to_string(&pid_path) {
        Ok(content) => Ok(content.trim().parse::<u32>().ok()),
        Err(_) => Ok(None),
    }
}

/// Execute kill command with the given signal and PID
fn kill_signal(signal: &str, pid: u32) -> bool {
    Command::new("kill")
        .args([signal, &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if a process with the given PID exists
pub fn process_exists(pid: u32) -> bool {
    kill_signal("-0", pid)
}

/// Force kill a daemon process
pub fn force_kill_daemon(pid: u32) -> bool {
    kill_signal("-9", pid)
}

/// Startup marker prefix that daemon writes to log before anything else.
const STARTUP_MARKER_PREFIX: &str = "--- ojd: starting (pid: ";

/// Read daemon log from startup marker, looking for errors.
///
/// Only reads the last 100KB of the file — the startup marker is always
/// within the last few KB of the current log.
pub fn read_startup_error() -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    let dir = daemon_dir().ok()?;
    let log_path = dir.join("daemon.log");

    let mut file = std::fs::File::open(&log_path).ok()?;
    let file_size = file.metadata().ok()?.len();

    // Seek to the last 100KB; the startup marker is always near the end.
    const TAIL_SIZE: u64 = 100 * 1024;
    if file_size > TAIL_SIZE {
        file.seek(SeekFrom::End(-(TAIL_SIZE as i64))).ok()?;
    }

    let mut content = String::new();
    file.read_to_string(&mut content).ok()?;
    parse_startup_error(&content)
}

/// Parse startup errors from log content (pure logic, no I/O).
fn parse_startup_error(content: &str) -> Option<String> {
    let start_pos = content.rfind(STARTUP_MARKER_PREFIX)?;
    let startup_log = &content[start_pos..];

    let errors: Vec<&str> = startup_log
        .lines()
        .filter(|line| line.contains(" ERROR ") || line.contains("Failed to start"))
        .collect();

    if errors.is_empty() {
        return None;
    }

    let error_messages: Vec<String> = errors
        .iter()
        .filter_map(|line| line.split_once(": ").map(|(_, msg)| msg.to_string()))
        .collect();

    if error_messages.is_empty() {
        Some(errors.join("\n"))
    } else {
        Some(error_messages.join("\n"))
    }
}

/// Wrap an error with startup log info if available.
pub fn wrap_with_startup_error(err: ClientError) -> ClientError {
    if matches!(err, ClientError::DaemonStartFailed(_)) {
        return err;
    }

    if let Some(startup_error) = read_startup_error() {
        ClientError::DaemonStartFailed(startup_error)
    } else {
        err
    }
}

/// Probe whether a Unix socket is accepting connections.
pub fn probe_socket(socket_path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket_path).is_ok()
}

/// Remove stale socket and PID files when the daemon is not running.
///
/// Called when the socket file exists but we can't connect to it.
/// If the PID file references a dead process (or no PID file exists),
/// removes stale files so a fresh daemon can start.
pub fn cleanup_stale_socket() -> Result<(), ClientError> {
    let dir = daemon_dir()?;
    let socket_path = dir.join("daemon.sock");
    let pid_path = dir.join("daemon.pid");

    if pid_path.exists() {
        if let Ok(Some(pid)) = read_daemon_pid() {
            if !process_exists(pid) {
                let _ = std::fs::remove_file(&socket_path);
                let _ = std::fs::remove_file(&pid_path);
            }
        } else {
            // PID file exists but can't read a valid PID - remove stale files
            let _ = std::fs::remove_file(&socket_path);
            let _ = std::fs::remove_file(&pid_path);
        }
    } else {
        // No PID file but socket exists - remove stale socket
        let _ = std::fs::remove_file(&socket_path);
    }

    Ok(())
}

#[cfg(test)]
#[path = "daemon_process_tests.rs"]
mod tests;
