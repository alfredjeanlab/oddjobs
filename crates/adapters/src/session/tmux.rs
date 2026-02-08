// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tmux session adapter

use super::{SessionAdapter, SessionError};
use crate::subprocess::{run_with_timeout, TMUX_TIMEOUT};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Tmux-based session adapter
#[derive(Clone, Default)]
pub struct TmuxAdapter {
    /// When set, spawned tmux commands use this directory for the socket
    /// (via `TMUX_TMPDIR`), isolating them from the user's default server.
    socket_dir: Option<PathBuf>,
}

impl TmuxAdapter {
    pub fn new() -> Self {
        Self { socket_dir: None }
    }

    /// Create an adapter that isolates tmux to the given socket directory.
    pub fn with_socket_dir(dir: PathBuf) -> Self {
        Self {
            socket_dir: Some(dir),
        }
    }

    /// Build a `Command` for tmux, applying `TMUX_TMPDIR` when configured.
    fn tmux_cmd(&self) -> Command {
        let mut cmd = Command::new("tmux");
        if let Some(ref dir) = self.socket_dir {
            cmd.env("TMUX_TMPDIR", dir);
        }
        cmd
    }

    /// Run a tmux command, returning `NotFound` on failure (discards output).
    async fn tmux_run(&self, args: &[&str], description: &str) -> Result<(), SessionError> {
        self.tmux_output(args, description).await.map(|_| ())
    }

    /// Run a tmux command and return the output, returning `NotFound` on failure.
    async fn tmux_output(
        &self,
        args: &[&str],
        description: &str,
    ) -> Result<std::process::Output, SessionError> {
        let mut cmd = self.tmux_cmd();
        cmd.args(args);
        let output = run_with_timeout(cmd, TMUX_TIMEOUT, description)
            .await
            .map_err(SessionError::CommandFailed)?;
        if !output.status.success() {
            let session_id = args
                .windows(2)
                .find(|w| w[0] == "-t")
                .map(|w| w[1])
                .unwrap_or("unknown");
            return Err(SessionError::NotFound(session_id.to_string()));
        }
        Ok(output)
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
        unset_env: &[String],
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
        let mut cmd_has = self.tmux_cmd();
        cmd_has.args(["has-session", "-t", &session_id]);
        let existing = run_with_timeout(cmd_has, TMUX_TIMEOUT, "tmux has-session").await;

        if existing.map(|o| o.status.success()).unwrap_or(false) {
            tracing::warn!(session_id, "session already exists, killing first");
            let mut cmd_kill = self.tmux_cmd();
            cmd_kill.args(["kill-session", "-t", &session_id]);
            let _ = run_with_timeout(cmd_kill, TMUX_TIMEOUT, "tmux kill-session").await;
        }

        // Build tmux command
        let mut tmux_cmd = self.tmux_cmd();
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

        if unset_env.is_empty() {
            tmux_cmd.arg(cmd);
        } else {
            let mut sanitized = String::from("env");
            for var in unset_env {
                sanitized.push_str(&format!(" -u {var}"));
            }
            sanitized.push(' ');
            sanitized.push_str(cmd);
            tmux_cmd.arg(sanitized);
        }

        let output = run_with_timeout(tmux_cmd, TMUX_TIMEOUT, "tmux new-session")
            .await
            .map_err(SessionError::SpawnFailed)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(
                session_id,
                stderr = %stderr,
                "tmux spawn failed"
            );
            return Err(SessionError::SpawnFailed(stderr.to_string()));
        }

        // Keep pane around after process exits so we can capture terminal
        // output (e.g. error messages) and exit codes before cleanup.
        let mut remain_cmd = self.tmux_cmd();
        remain_cmd.args(["set-option", "-t", &session_id, "remain-on-exit", "on"]);
        let _ = run_with_timeout(remain_cmd, TMUX_TIMEOUT, "tmux set remain-on-exit").await;

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
        self.tmux_run(&["send-keys", "-t", id, input], "tmux send-keys")
            .await
    }

    async fn send_literal(&self, id: &str, text: &str) -> Result<(), SessionError> {
        // -l = literal mode (no key name interpretation)
        // -- = end of options (handles text starting with -)
        self.tmux_run(
            &["send-keys", "-t", id, "-l", "--", text],
            "tmux send-keys literal",
        )
        .await
    }

    async fn send_enter(&self, id: &str) -> Result<(), SessionError> {
        self.tmux_run(&["send-keys", "-t", id, "Enter"], "tmux send-keys enter")
            .await
    }

    async fn kill(&self, id: &str) -> Result<(), SessionError> {
        // Ignore failure — session might already be dead, which is fine
        let mut cmd = self.tmux_cmd();
        cmd.args(["kill-session", "-t", id]);
        let _ = run_with_timeout(cmd, TMUX_TIMEOUT, "tmux kill-session").await;
        Ok(())
    }

    async fn is_alive(&self, id: &str) -> Result<bool, SessionError> {
        let mut cmd = self.tmux_cmd();
        cmd.args(["has-session", "-t", id]);
        let output = run_with_timeout(cmd, TMUX_TIMEOUT, "tmux has-session")
            .await
            .map_err(SessionError::CommandFailed)?;
        Ok(output.status.success())
    }

    async fn capture_output(&self, id: &str, lines: u32) -> Result<String, SessionError> {
        let lines_arg = format!("-{}", lines);
        let output = self
            .tmux_output(
                &["capture-pane", "-t", id, "-p", "-S", &lines_arg],
                "tmux capture-pane",
            )
            .await?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn is_process_running(&self, id: &str, pattern: &str) -> Result<bool, SessionError> {
        let output = self
            .tmux_output(
                &["list-panes", "-t", id, "-F", "#{pane_pid}"],
                "tmux list-panes",
            )
            .await?;

        let pane_pid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if pane_pid.is_empty() {
            return Ok(false);
        }

        // Run both checks concurrently: the pane process itself and its children.
        let (ps_output, pgrep_output) = tokio::try_join!(
            async {
                let mut cmd = Command::new("ps");
                cmd.args(["-p", &pane_pid, "-o", "command="]);
                run_with_timeout(cmd, TMUX_TIMEOUT, "ps pane check")
                    .await
                    .map_err(SessionError::CommandFailed)
            },
            async {
                let mut cmd = Command::new("pgrep");
                cmd.args(["-P", &pane_pid, "-f", pattern]);
                run_with_timeout(cmd, TMUX_TIMEOUT, "pgrep child check")
                    .await
                    .map_err(SessionError::CommandFailed)
            },
        )?;

        if ps_output.status.success() {
            let cmd_line = String::from_utf8_lossy(&ps_output.stdout);
            if cmd_line.contains(pattern) {
                return Ok(true);
            }
        }

        Ok(pgrep_output.status.success())
    }

    async fn get_exit_code(&self, id: &str) -> Result<Option<i32>, SessionError> {
        let output = self
            .tmux_output(
                &["display-message", "-t", id, "-p", "#{pane_dead_status}"],
                "tmux display-message",
            )
            .await?;

        let status_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(status_str.parse::<i32>().ok())
    }
}

#[cfg(test)]
#[path = "tmux_tests.rs"]
mod tests;
