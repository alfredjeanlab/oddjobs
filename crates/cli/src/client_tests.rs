// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for daemon client behavior.

use super::{ClientError, DaemonClient};
use crate::client_lifecycle::log_connection_error;
use crate::daemon_process::{cleanup_stale_socket, daemon_dir, probe_socket};
use serial_test::serial;
use std::fs;
use tempfile::tempdir;

/// Verify that connect() does not delete state files when daemon is not running.
///
/// This is a regression test for a race condition where connect() would call
/// cleanup_stale_files() during startup polling, deleting the pid file before
/// the daemon finished initializing.
#[test]
#[serial]
fn connect_does_not_delete_pid_file() {
    // Set up isolated state directory
    let state_dir = tempdir().unwrap();
    std::env::set_var("XDG_STATE_HOME", state_dir.path());

    // Create a pid file (simulating daemon mid-startup)
    let dir = daemon_dir().unwrap();
    fs::create_dir_all(&dir).unwrap();
    let pid_path = dir.join("daemon.pid");
    fs::write(&pid_path, "12345\n").unwrap();

    // connect() should fail (no socket) but NOT delete the pid file
    let result = DaemonClient::connect();
    assert!(matches!(result, Err(ClientError::DaemonNotRunning)));

    // Pid file should still exist
    assert!(pid_path.exists(), "connect() must not delete pid file");
}

/// Verify log_connection_error creates cli.log with expected format.
#[test]
#[serial] // Tests modify OJ_STATE_DIR env var which is process-wide
fn log_connection_error_creates_log_file() {
    let state_dir = tempdir().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let error = ClientError::DaemonNotRunning;
    log_connection_error(&error);

    let log_path = state_dir.path().join("cli.log");
    assert!(log_path.exists(), "cli.log should be created");

    let content = fs::read_to_string(&log_path).unwrap();
    assert!(content.contains("pid="), "log should contain pid");
    assert!(content.contains("cwd="), "log should contain cwd");
    assert!(
        content.contains("OJ_STATE_DIR="),
        "log should contain OJ_STATE_DIR"
    );
    assert!(
        content.contains("socket="),
        "log should contain socket path"
    );
    assert!(
        content.contains("Daemon not running"),
        "log should contain error message"
    );
}

/// Verify log_connection_error includes socket path in output.
#[test]
#[serial]
fn log_connection_error_includes_socket_path() {
    let state_dir = tempdir().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let error = ClientError::DaemonNotRunning;
    log_connection_error(&error);

    let log_path = state_dir.path().join("cli.log");
    let content = fs::read_to_string(&log_path).unwrap();

    // Should include the socket path we're looking for
    let expected_socket = state_dir.path().join("daemon.sock");
    assert!(
        content.contains(&expected_socket.display().to_string()),
        "log should contain expected socket path"
    );
}

/// Verify stale socket and PID files are cleaned up when daemon process is dead.
///
/// Simulates a crashed daemon: socket file exists, PID file references a dead process.
/// cleanup_stale_socket should remove both files.
#[test]
#[serial]
fn test_stale_socket_cleanup() {
    let state_dir = tempdir().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    // Create stale socket file
    let socket_path = state_dir.path().join("daemon.sock");
    fs::write(&socket_path, "").unwrap();

    // Get a dead PID by spawning a short-lived process
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = child.id();
    child.wait().unwrap();

    // Create PID file with the dead PID
    let pid_path = state_dir.path().join("daemon.pid");
    fs::write(&pid_path, format!("{}\n", dead_pid)).unwrap();

    // Socket file is not a real Unix socket, so probe should fail
    assert!(!probe_socket(&socket_path));

    // Cleanup should remove both stale files
    cleanup_stale_socket().unwrap();

    assert!(!socket_path.exists(), "stale socket should be removed");
    assert!(!pid_path.exists(), "stale PID file should be removed");
}

/// Verify stale socket is cleaned up when no PID file exists.
///
/// If the socket file exists but there's no PID file at all, the socket is
/// definitely stale (daemon can't be running without a PID file).
#[test]
#[serial]
fn test_stale_lock_cleanup() {
    let state_dir = tempdir().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    // Create stale socket file (no PID file)
    let socket_path = state_dir.path().join("daemon.sock");
    fs::write(&socket_path, "").unwrap();

    let pid_path = state_dir.path().join("daemon.pid");
    assert!(
        !pid_path.exists(),
        "PID file should not exist for this test"
    );

    // Socket file is not a real Unix socket, so probe should fail
    assert!(!probe_socket(&socket_path));

    // Cleanup should remove the stale socket
    cleanup_stale_socket().unwrap();

    assert!(!socket_path.exists(), "stale socket should be removed");
}
