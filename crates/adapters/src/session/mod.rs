// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session management adapters

mod noop;
mod tmux;

pub use noop::NoOpSessionAdapter;
pub use tmux::TmuxAdapter;

// Test support - only compiled for tests or when explicitly requested
#[cfg(any(test, feature = "test-support"))]
mod fake;
#[cfg(any(test, feature = "test-support"))]
pub use fake::{FakeSession, FakeSessionAdapter, SessionCall};

use async_trait::async_trait;
use std::path::Path;
use thiserror::Error;

/// Errors from session operations
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("spawn failed: {0}")]
    SpawnFailed(String),
    #[error("command failed: {0}")]
    CommandFailed(String),
}

/// Adapter for managing terminal sessions (tmux, etc.)
#[async_trait]
pub trait SessionAdapter: Clone + Send + Sync + 'static {
    /// Spawn a new session
    async fn spawn(
        &self,
        name: &str,
        cwd: &Path,
        cmd: &str,
        env: &[(String, String)],
        unset_env: &[String],
    ) -> Result<String, SessionError>;

    /// Send input to a session
    async fn send(&self, id: &str, input: &str) -> Result<(), SessionError>;

    /// Send literal text to a session (no key interpretation)
    async fn send_literal(&self, id: &str, text: &str) -> Result<(), SessionError>;

    /// Send the Enter key to a session
    async fn send_enter(&self, id: &str) -> Result<(), SessionError>;

    /// Kill a session
    async fn kill(&self, id: &str) -> Result<(), SessionError>;

    /// Check if a session is alive
    async fn is_alive(&self, id: &str) -> Result<bool, SessionError>;

    /// Capture recent output from a session
    async fn capture_output(&self, id: &str, lines: u32) -> Result<String, SessionError>;

    /// Check if a process matching pattern is running inside the session
    async fn is_process_running(&self, id: &str, pattern: &str) -> Result<bool, SessionError>;

    /// Get the exit code of the pane's process (if available)
    ///
    /// Returns `None` if the pane is still running or the exit code is unavailable.
    async fn get_exit_code(&self, id: &str) -> Result<Option<i32>, SessionError>;

    /// Apply configuration to an existing session (styling, status bar, etc.)
    /// Default implementation is a no-op.
    async fn configure(&self, _id: &str, _config: &serde_json::Value) -> Result<(), SessionError> {
        Ok(())
    }
}
