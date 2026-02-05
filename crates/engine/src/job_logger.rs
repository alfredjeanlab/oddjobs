// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Append-only logger for per-job activity logs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::log_paths;
use crate::time_fmt::format_utc_now;

/// Append-only logger for per-job activity logs.
///
/// Writes human-readable timestamped lines to:
///   `<log_dir>/job/<job_id>.log`
///
/// Each `append()` call opens, writes, and closes the file.
/// This is safe for the low write frequency of job events.
pub struct JobLogger {
    log_dir: PathBuf,
}

impl JobLogger {
    pub fn new(log_dir: PathBuf) -> Self {
        Self { log_dir }
    }

    /// Returns the base log directory path.
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// Append a log line for the given job.
    ///
    /// Format: `2026-01-30T08:14:09Z [step] message`
    ///
    /// Failures are logged via tracing but do not propagate — logging
    /// must not break the engine.
    pub fn append(&self, job_id: &str, step: &str, message: &str) {
        let path = log_paths::job_log_path(&self.log_dir, job_id);
        if let Err(e) = self.write_line(&path, step, message) {
            tracing::warn!(
                job_id,
                error = %e,
                "failed to write job log"
            );
        }
    }

    /// Append a pointer line to the agent log for a step.
    ///
    /// Format: `2026-01-30T08:17:00Z [step] agent log: /full/path/to/logs/agent/<agent_id>.log`
    pub fn append_agent_pointer(&self, job_id: &str, step: &str, agent_id: &str) {
        let log_path = log_paths::agent_log_path(&self.log_dir, agent_id);
        let message = format!("agent log: {}", log_path.display());
        self.append(job_id, step, &message);
    }

    /// Copy the agent's session.jsonl to the logs directory.
    ///
    /// Copies the source file to `{logs_dir}/agent/{agent_id}/session.jsonl`.
    /// Failures are logged via tracing but do not propagate.
    pub fn copy_session_log(&self, agent_id: &str, source: &Path) {
        let dest_dir = log_paths::agent_session_log_dir(&self.log_dir, agent_id);
        let dest = dest_dir.join("session.jsonl");

        if let Err(e) = fs::create_dir_all(&dest_dir) {
            tracing::warn!(
                agent_id,
                error = %e,
                "failed to create session log directory"
            );
            return;
        }

        if let Err(e) = fs::copy(source, &dest) {
            tracing::warn!(
                agent_id,
                source = %source.display(),
                dest = %dest.display(),
                error = %e,
                "failed to copy session log"
            );
        } else {
            tracing::debug!(
                agent_id,
                dest = %dest.display(),
                "copied session log"
            );
        }
    }

    /// Append a fenced block to the job log.
    ///
    /// Format:
    /// ```text
    /// {timestamp} [{step}] ```{label}
    /// {content}
    /// {timestamp} [{step}] ```
    /// ```
    pub fn append_fenced(&self, job_id: &str, step: &str, label: &str, content: &str) {
        let path = log_paths::job_log_path(&self.log_dir, job_id);
        if let Err(e) = self.write_fenced(&path, step, label, content) {
            tracing::warn!(
                job_id,
                error = %e,
                "failed to write job log"
            );
        }
    }

    fn write_fenced(
        &self,
        path: &Path,
        step: &str,
        label: &str,
        content: &str,
    ) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let ts = format_utc_now();
        writeln!(file, "{} [{}] ```{}", ts, step, label)?;
        write!(file, "{}", content)?;
        if !content.ends_with('\n') {
            writeln!(file)?;
        }
        let ts = format_utc_now();
        writeln!(file, "{} [{}] ```", ts, step)?;
        Ok(())
    }

    fn write_line(&self, path: &Path, step: &str, message: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let ts = format_utc_now();
        writeln!(file, "{} [{}] {}", ts, step, message)?;
        Ok(())
    }

    /// Append a spawn error to an agent's log file.
    ///
    /// Format: `2026-01-30T08:14:09Z error: <message>`
    ///
    /// This is used when agent spawn fails before the watcher is started,
    /// so there's no other mechanism to write to the agent log.
    pub fn append_agent_error(&self, agent_id: &str, message: &str) {
        let path = log_paths::agent_log_path(&self.log_dir, agent_id);
        if let Err(e) = self.write_agent_error(&path, message) {
            tracing::warn!(
                agent_id,
                error = %e,
                "failed to write agent spawn error log"
            );
        }
    }

    fn write_agent_error(&self, path: &Path, message: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let ts = format_utc_now();
        writeln!(file, "{} error: {}", ts, message)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "job_logger_tests.rs"]
mod tests;
