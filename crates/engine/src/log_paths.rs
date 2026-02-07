// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared path builders for job, agent, and worker log files.
//!
//! Used by both the logger (writer) and daemon (reader) to construct
//! consistent paths for log files in the directory structure:
//!   `<logs_dir>/job/<job_id>.log`
//!   `<logs_dir>/agent/<agent_id>.log`
//!   `<logs_dir>/worker/<worker_name>.log`

use std::path::{Path, PathBuf};

/// Build the path to a job log file.
///
/// Structure: `{logs_dir}/job/{job_id}.log`
///
/// # Arguments
/// * `logs_dir` - Base logs directory (e.g., `~/.local/state/oj/logs`)
/// * `job_id` - Job identifier
pub fn job_log_path(logs_dir: &Path, job_id: &str) -> PathBuf {
    logs_dir.join("job").join(format!("{}.log", job_id))
}

/// Build the path to an agent log file.
///
/// Structure: `{logs_dir}/agent/{agent_id}.log`
///
/// # Arguments
/// * `logs_dir` - Base logs directory (e.g., `~/.local/state/oj/logs`)
/// * `agent_id` - Agent UUID
pub fn agent_log_path(logs_dir: &Path, agent_id: &str) -> PathBuf {
    logs_dir.join("agent").join(format!("{}.log", agent_id))
}

/// Build the path to an agent's session log directory.
///
/// Structure: `{logs_dir}/agent/{agent_id}/`
///
/// # Arguments
/// * `logs_dir` - Base logs directory (e.g., `~/.local/state/oj/logs`)
/// * `agent_id` - Agent UUID
pub fn agent_session_log_dir(logs_dir: &Path, agent_id: &str) -> PathBuf {
    logs_dir.join("agent").join(agent_id)
}

/// Build the path to a cron log file.
///
/// Structure: `{logs_dir}/cron/{cron_name}.log`
pub fn cron_log_path(logs_dir: &Path, cron_name: &str) -> PathBuf {
    logs_dir.join("cron").join(format!("{}.log", cron_name))
}

/// Build the path to a worker log file.
///
/// Structure: `{logs_dir}/worker/{worker_name}.log`
///
/// # Arguments
/// * `logs_dir` - Base logs directory (e.g., `~/.local/state/oj/logs`)
/// * `worker_name` - Worker name (may include namespace prefix, e.g., `ns/worker`)
pub fn worker_log_path(logs_dir: &Path, worker_name: &str) -> PathBuf {
    logs_dir.join("worker").join(format!("{}.log", worker_name))
}

/// Build the path to a queue's activity log file.
///
/// Structure: `{logs_dir}/queue/{queue_name}.log`
///
/// The `queue_name` may contain `/` (e.g. `namespace/queue_name`),
/// in which case `Path::join` creates nested directories automatically.
///
/// # Arguments
/// * `logs_dir` - Base logs directory (e.g., `~/.local/state/oj/logs`)
/// * `queue_name` - Queue name (possibly namespace-scoped like `ns/queue`)
pub fn queue_log_path(logs_dir: &Path, queue_name: &str) -> PathBuf {
    logs_dir.join("queue").join(format!("{}.log", queue_name))
}

/// Build the path to an agent's terminal capture file.
///
/// Structure: `{logs_dir}/agent/{agent_id}/capture.latest.txt`
///
/// # Arguments
/// * `logs_dir` - Base logs directory (e.g., `~/.local/state/oj/logs`)
/// * `agent_id` - Agent UUID
pub fn agent_capture_path(logs_dir: &Path, agent_id: &str) -> PathBuf {
    logs_dir
        .join("agent")
        .join(agent_id)
        .join("capture.latest.txt")
}

/// Build the path to a job breadcrumb file.
///
/// Structure: `{logs_dir}/{job_id}.crumb.json`
pub fn breadcrumb_path(logs_dir: &Path, job_id: &str) -> PathBuf {
    logs_dir.join(format!("{}.crumb.json", job_id))
}

/// Build the path to the usage metrics JSONL file.
///
/// Structure: `{state_dir}/metrics/usage.jsonl`
///
/// # Arguments
/// * `state_dir` - Root state directory (e.g., `~/.local/state/oj`)
pub fn metrics_usage_path(state_dir: &Path) -> PathBuf {
    state_dir.join("metrics").join("usage.jsonl")
}

#[cfg(test)]
#[path = "log_paths_tests.rs"]
mod tests;
