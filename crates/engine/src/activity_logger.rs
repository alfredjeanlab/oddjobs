// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unified append-only logger for per-entity activity logs.
//!
//! Provides a single parameterized type `ActivityLogger<K>` that handles
//! job, worker, and queue logging with type-specific behavior determined
//! by the `LogKind` marker trait.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use oj_core::ShortId;

use crate::log_paths;
use crate::time_fmt::format_utc_now;

/// Marker trait for activity log kinds.
///
/// Implementations define the subdirectory and tracing field name for each log type.
pub trait LogKind {
    /// Subdirectory within logs dir (e.g., "job", "worker", "queue").
    const SUBDIR: &'static str;
}

/// Marker type for job logs.
pub struct JobLog;
impl LogKind for JobLog {
    const SUBDIR: &'static str = "job";
}

/// Marker type for worker logs.
pub struct WorkerLog;
impl LogKind for WorkerLog {
    const SUBDIR: &'static str = "worker";
}

/// Marker type for queue logs.
pub struct QueueLog;
impl LogKind for QueueLog {
    const SUBDIR: &'static str = "queue";
}

/// Unified append-only logger for per-entity activity logs.
///
/// Writes human-readable timestamped lines to:
///   `<log_dir>/<subdir>/<entity_id>.log`
///
/// Each `append()` call opens, writes, and closes the file.
/// This is safe for the low write frequency of activity events.
pub struct ActivityLogger<K: LogKind> {
    log_dir: PathBuf,
    _kind: PhantomData<K>,
}

impl<K: LogKind> ActivityLogger<K> {
    /// Create a new activity logger.
    pub fn new(log_dir: PathBuf) -> Self {
        Self {
            log_dir,
            _kind: PhantomData,
        }
    }

    /// Write a timestamped line to a log file.
    ///
    /// Format: `YYYY-MM-DDTHH:MM:SSZ [{label}] {message}`
    fn write_line(&self, path: &Path, label: &str, message: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let ts = format_utc_now();
        writeln!(file, "{} [{}] {}", ts, label, message)?;
        Ok(())
    }
}

// =============================================================================
// JobLogger implementation
// =============================================================================

/// Type alias for job activity logger.
pub type JobLogger = ActivityLogger<JobLog>;

impl ActivityLogger<JobLog> {
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

// =============================================================================
// WorkerLogger implementation
// =============================================================================

/// Type alias for worker activity logger.
pub type WorkerLogger = ActivityLogger<WorkerLog>;

impl ActivityLogger<WorkerLog> {
    /// Append a log line for the given worker.
    ///
    /// Format: `2026-01-30T08:14:09Z [worker] message`
    ///
    /// Failures are logged via tracing but do not propagate — logging
    /// must not break the engine.
    pub fn append(&self, worker_name: &str, message: &str) {
        let path = log_paths::worker_log_path(&self.log_dir, worker_name);
        if let Err(e) = self.write_line(&path, "worker", message) {
            tracing::warn!(
                worker_name,
                error = %e,
                "failed to write worker log"
            );
        }
    }
}

// =============================================================================
// QueueLogger implementation
// =============================================================================

/// Type alias for queue activity logger.
pub type QueueLogger = ActivityLogger<QueueLog>;

impl ActivityLogger<QueueLog> {
    /// Append a timestamped log line to the queue's log file.
    ///
    /// Format: `2026-01-30T08:14:09Z [item_id_prefix] message`
    ///
    /// Failures are logged via tracing but do not propagate — logging
    /// must not break the engine.
    pub fn append(&self, queue_name: &str, item_id: &str, message: &str) {
        let path = log_paths::queue_log_path(&self.log_dir, queue_name);
        let prefix = item_id.short(8);
        if let Err(e) = self.write_line(&path, prefix, message) {
            tracing::warn!(
                queue_name,
                error = %e,
                "failed to write queue log"
            );
        }
    }
}

#[cfg(test)]
#[path = "activity_logger_tests.rs"]
mod tests;
