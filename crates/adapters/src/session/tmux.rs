// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tmux session adapter

use super::{SessionAdapter, SessionError};
use async_trait::async_trait;
use std::path::Path;
use tokio::process::Command;

/// Tmux-based session adapter
#[derive(Clone, Default)]
pub struct TmuxAdapter;

impl TmuxAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SessionAdapter for TmuxAdapter {
    async fn spawn(
        &self,
        name: &str,
        cwd: &Path,
        cmd: &str,
        env: &[(String, String)],
    ) -> Result<String, SessionError> {
        // Precondition: cwd must exist
        if !cwd.exists() {
            return Err(SessionError::SpawnFailed(format!(
                "working directory does not exist: {}",
                cwd.display()
            )));
        }

        let session_id = format!("oj-{}", name);

        // Check if session already exists and clean it up
        let existing = Command::new("tmux")
            .args(["has-session", "-t", &session_id])
            .output()
            .await;

        if existing.map(|o| o.status.success()).unwrap_or(false) {
            tracing::warn!(session_id, "session already exists, killing first");
            let _ = Command::new("tmux")
                .args(["kill-session", "-t", &session_id])
                .output()
                .await;
        }

        // Build tmux command
        let mut tmux_cmd = Command::new("tmux");
        tmux_cmd
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&session_id)
            .arg("-c")
            .arg(cwd);

        // Add environment variables
        for (key, value) in env {
            tmux_cmd.arg("-e").arg(format!("{}={}", key, value));
        }

        tmux_cmd.arg(cmd);

        let output = tmux_cmd
            .output()
            .await
            .map_err(|e| SessionError::SpawnFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(
                session_id,
                stderr = %stderr,
                "tmux spawn failed"
            );
            return Err(SessionError::SpawnFailed(stderr.to_string()));
        }

        // Log stderr even on success - may contain useful warnings
        if !output.stderr.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                session_id,
                stderr = %stderr,
                "tmux spawn stderr (non-fatal)"
            );
        }

        Ok(session_id)
    }

    async fn send(&self, id: &str, input: &str) -> Result<(), SessionError> {
        let output = Command::new("tmux")
            .arg("send-keys")
            .arg("-t")
            .arg(id)
            .arg(input)
            .output()
            .await
            .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(SessionError::NotFound(id.to_string()));
        }

        Ok(())
    }

    async fn kill(&self, id: &str) -> Result<(), SessionError> {
        let output = Command::new("tmux")
            .arg("kill-session")
            .arg("-t")
            .arg(id)
            .output()
            .await
            .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

        if !output.status.success() {
            // Session might already be dead, which is fine
        }

        Ok(())
    }

    async fn is_alive(&self, id: &str) -> Result<bool, SessionError> {
        let output = Command::new("tmux")
            .arg("has-session")
            .arg("-t")
            .arg(id)
            .output()
            .await
            .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

        Ok(output.status.success())
    }

    async fn capture_output(&self, id: &str, lines: u32) -> Result<String, SessionError> {
        let output = Command::new("tmux")
            .arg("capture-pane")
            .arg("-t")
            .arg(id)
            .arg("-p")
            .arg("-S")
            .arg(format!("-{}", lines))
            .output()
            .await
            .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(SessionError::NotFound(id.to_string()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn is_process_running(&self, id: &str, pattern: &str) -> Result<bool, SessionError> {
        // Get the pane PID
        let output = Command::new("tmux")
            .args(["list-panes", "-t", id, "-F", "#{pane_pid}"])
            .output()
            .await
            .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(SessionError::NotFound(id.to_string()));
        }

        let pane_pid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if pane_pid.is_empty() {
            return Ok(false);
        }

        // Check if the pane process itself matches the pattern.
        // When tmux launches a command directly (e.g., `tmux new-session -d -s id cmd`),
        // the command IS the pane process — not a child of it.
        let ps_output = Command::new("ps")
            .args(["-p", &pane_pid, "-o", "command="])
            .output()
            .await
            .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

        if ps_output.status.success() {
            let cmd_line = String::from_utf8_lossy(&ps_output.stdout);
            if cmd_line.contains(pattern) {
                return Ok(true);
            }
        }

        // Also check child processes matching pattern (e.g., when run via a shell)
        let output = Command::new("pgrep")
            .args(["-P", &pane_pid, "-f", pattern])
            .output()
            .await
            .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

        Ok(output.status.success())
    }

    async fn get_exit_code(&self, id: &str) -> Result<Option<i32>, SessionError> {
        // Query the pane's dead status (exit code when process has exited)
        let output = Command::new("tmux")
            .args(["display-message", "-t", id, "-p", "#{pane_dead_status}"])
            .output()
            .await
            .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(SessionError::NotFound(id.to_string()));
        }

        let status_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if status_str.is_empty() {
            // Process is still running
            return Ok(None);
        }

        // Parse exit code
        match status_str.parse::<i32>() {
            Ok(code) => Ok(Some(code)),
            Err(_) => Ok(None),
        }
    }
}

#[cfg(test)]
#[path = "tmux_tests.rs"]
mod tests;
