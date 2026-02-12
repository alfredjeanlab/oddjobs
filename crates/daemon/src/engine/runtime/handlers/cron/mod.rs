// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron event handling

mod lifecycle;
mod timer;

use crate::engine::time_fmt::format_utc_now;
use oj_core::log_paths::cron_log_path;
use oj_core::{scoped_name, JobId, OwnerId, RunTarget};
use std::io::Write;
use std::path::{Path, PathBuf};

/// In-memory state for a running cron
pub(crate) struct CronState {
    pub project_path: PathBuf,
    pub runbook_hash: String,
    pub interval: String,
    pub target: RunTarget,
    pub status: CronStatus,
    pub project: String,
    /// Maximum concurrent jobs this cron can have running. Default 1.
    pub concurrency: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CronStatus {
    Running,
    Stopped,
}

/// Append a timestamped line to the cron log file.
///
/// Creates the `{logs_dir}/cron/` directory on first write.
/// Errors are silently ignored — logging must not break the cron.
pub(super) fn append_cron_log(logs_dir: &Path, cron_name: &str, project: &str, message: &str) {
    let scoped = scoped_name(project, cron_name);
    let path = cron_log_path(logs_dir, &scoped);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let ts = format_utc_now();
        let _ = writeln!(f, "[{}] {}", ts, message);
    }
}

/// Parameters for handling a cron started event.
pub(crate) struct CronStartedParams<'a> {
    pub cron: &'a str,
    pub project: &'a str,
    pub project_path: &'a Path,
    pub runbook_hash: &'a str,
    pub interval: &'a str,
    pub target: &'a RunTarget,
}

/// Parameters for handling a one-shot cron execution.
pub(crate) struct CronOnceParams<'a> {
    pub cron: &'a str,
    pub owner: &'a OwnerId,
    pub project: &'a str,
    pub project_path: &'a Path,
    pub runbook_hash: &'a str,
    pub target: &'a RunTarget,
}

/// Parameters for creating an inline cron shell job.
pub(crate) struct CronShellJobParams<'a> {
    pub cron: &'a str,
    pub project: &'a str,
    pub job_id: JobId,
    pub job_display: &'a str,
    pub runbook_hash: &'a str,
    pub cmd: &'a str,
    pub cwd: &'a Path,
}
