// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Centralized environment variable access for the CLI crate.

use std::path::PathBuf;
use std::time::Duration;

use crate::client::ClientError;

// --- Duration helper (private) ---

fn parse_duration_ms(var: &str) -> Option<Duration> {
    std::env::var(var).ok().and_then(|s| s.parse::<u64>().ok()).map(Duration::from_millis)
}

// --- State directory ---

/// Resolve state directory: OJ_STATE_DIR > XDG_STATE_HOME/oj > ~/.local/state/oj
pub fn state_dir() -> Result<PathBuf, ClientError> {
    if let Ok(dir) = std::env::var("OJ_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        return Ok(PathBuf::from(xdg).join("oj"));
    }
    let home = std::env::var("HOME").map_err(|_| ClientError::NoStateDir)?;
    Ok(PathBuf::from(home).join(".local/state/oj"))
}

/// Read OJ_STATE_DIR raw (for diagnostic logging)
pub fn state_dir_raw() -> Option<String> {
    std::env::var("OJ_STATE_DIR").ok()
}

// --- Namespace ---

pub fn project() -> Option<String> {
    std::env::var("OJ_PROJECT").ok().filter(|s| !s.is_empty())
}

// --- Color ---

pub fn no_color() -> bool {
    std::env::var("NO_COLOR").is_ok_and(|v| v == "1")
}

pub fn force_color() -> bool {
    std::env::var("COLOR").is_ok_and(|v| v == "1")
}

// --- Daemon connection ---

/// Remote daemon URL (e.g. `tcp://host:7777`). When set, CLI connects via TCP
/// instead of the local Unix socket.
pub fn daemon_url() -> Option<String> {
    std::env::var("OJ_DAEMON_URL").ok().filter(|s| !s.is_empty())
}

/// Auth token for TCP connections to the daemon.
pub fn auth_token() -> Option<String> {
    std::env::var("OJ_AUTH_TOKEN").ok().filter(|s| !s.is_empty())
}

// --- Daemon binary ---

pub fn daemon_binary() -> Option<String> {
    std::env::var("OJ_DAEMON_BINARY").ok()
}

pub fn cargo_manifest_dir() -> Option<String> {
    std::env::var("CARGO_MANIFEST_DIR").ok()
}

// --- Timeouts ---

pub fn timeout_ipc_ms() -> Option<Duration> {
    parse_duration_ms("OJ_TIMEOUT_IPC_MS")
}
pub fn timeout_connect_ms() -> Option<Duration> {
    parse_duration_ms("OJ_TIMEOUT_CONNECT_MS")
}
pub fn timeout_exit_ms() -> Option<Duration> {
    parse_duration_ms("OJ_TIMEOUT_EXIT_MS")
}
pub fn connect_poll_ms() -> Option<Duration> {
    parse_duration_ms("OJ_CONNECT_POLL_MS")
}
pub fn wait_poll_ms() -> Option<Duration> {
    parse_duration_ms("OJ_WAIT_POLL_MS")
}
pub fn run_wait_ms() -> Option<Duration> {
    parse_duration_ms("OJ_RUN_WAIT_MS")
}
