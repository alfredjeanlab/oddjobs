// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Types and helpers for cron event handling.

use crate::log_paths::cron_log_path;
use crate::time_fmt::format_utc_now;
use oj_core::{scoped_name, JobId};
use std::path::{Path, PathBuf};

/// What a cron targets when it fires.
#[derive(Debug, Clone)]
pub(crate) enum CronRunTarget {
    Job(String),
    Agent(String),
    Shell(String),
}

impl CronRunTarget {
    /// Parse a "job:name", "agent:name", or "shell:cmd" string.
    pub(crate) fn from_run_target_str(s: &str) -> Self {
        if let Some(name) = s.strip_prefix("agent:") {
            CronRunTarget::Agent(name.to_string())
        } else if let Some(cmd) = s.strip_prefix("shell:") {
            CronRunTarget::Shell(cmd.to_string())
        } else if let Some(name) = s.strip_prefix("job:") {
            CronRunTarget::Job(name.to_string())
        } else {
            // Backward compat: bare name = job
            CronRunTarget::Job(s.to_string())
        }
    }

    /// Get the display name for logging.
    pub(crate) fn display_name(&self) -> String {
        match self {
            CronRunTarget::Job(name) => format!("job={}", name),
            CronRunTarget::Agent(name) => format!("agent={}", name),
            CronRunTarget::Shell(cmd) => format!("shell={}", cmd),
        }
    }
}

/// In-memory state for a running cron
pub(crate) struct CronState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub interval: String,
    pub run_target: CronRunTarget,
    pub status: CronStatus,
    pub namespace: String,
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
pub(super) fn append_cron_log(logs_dir: &Path, cron_name: &str, namespace: &str, message: &str) {
    let scoped = scoped_name(namespace, cron_name);
    let path = cron_log_path(logs_dir, &scoped);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let ts = format_utc_now();
        let _ = writeln!(f, "[{}] {}", ts, message);
    }
}

/// Parameters for handling a cron started event.
pub(crate) struct CronStartedParams<'a> {
    pub cron_name: &'a str,
    pub project_root: &'a Path,
    pub runbook_hash: &'a str,
    pub interval: &'a str,
    pub run_target: &'a str,
    pub namespace: &'a str,
}

/// Parameters for handling a one-shot cron execution.
pub(crate) struct CronOnceParams<'a> {
    pub cron_name: &'a str,
    pub job_id: &'a JobId,
    pub job_name: &'a str,
    pub job_kind: &'a str,
    pub agent_run_id: &'a Option<String>,
    pub agent_name: &'a Option<String>,
    pub runbook_hash: &'a str,
    pub run_target: &'a str,
    pub namespace: &'a str,
    pub project_root: &'a Path,
}

/// Parameters for creating an inline cron shell job.
pub(crate) struct CronShellJobParams<'a> {
    pub job_id: JobId,
    pub cron_name: &'a str,
    pub job_display: &'a str,
    pub cmd: &'a str,
    pub runbook_hash: &'a str,
    pub namespace: &'a str,
    pub cwd: &'a Path,
}
