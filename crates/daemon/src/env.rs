// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Centralized environment variable access for the daemon crate.

use std::path::PathBuf;
use std::time::Duration;

use crate::lifecycle::LifecycleError;

/// Protocol version (from Cargo.toml)
pub const PROTOCOL_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH"));

/// Resolve state directory: OJ_STATE_DIR > XDG_STATE_HOME/oj > ~/.local/state/oj
pub fn state_dir() -> Result<PathBuf, LifecycleError> {
    if let Ok(dir) = std::env::var("OJ_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        return Ok(PathBuf::from(xdg).join("oj"));
    }
    let home = std::env::var("HOME").map_err(|_| LifecycleError::NoStateDir)?;
    Ok(PathBuf::from(home).join(".local/state/oj"))
}

/// Default IPC timeout
pub fn ipc_timeout() -> Duration {
    std::env::var("OJ_IPC_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(5))
}

/// TCP port for remote connections. When set, the daemon listens on this port
/// in addition to the Unix socket. Used for Kubernetes and Docker deployments.
pub fn tcp_port() -> Option<u16> {
    std::env::var("OJ_TCP_PORT").ok().and_then(|s| s.parse::<u16>().ok())
}

/// Auth token for TCP connections. Required when `OJ_TCP_PORT` is set.
/// Validated in the Hello handshake for TCP connections.
pub fn auth_token() -> Option<String> {
    std::env::var("OJ_AUTH_TOKEN").ok().filter(|s| !s.is_empty())
}

/// Shutdown drain timeout (default 5s, configurable via `OJ_DRAIN_TIMEOUT_MS`).
pub fn drain_timeout() -> Duration {
    std::env::var("OJ_DRAIN_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(5))
}

/// Timer check interval override
pub fn timer_check_ms() -> Option<Duration> {
    std::env::var("OJ_TIMER_CHECK_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
}
