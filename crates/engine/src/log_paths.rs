// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared path builders for pipeline, agent, and worker log files.
//!
//! Used by both the logger (writer) and daemon (reader) to construct
//! consistent paths for log files in the directory structure:
//!   `<logs_dir>/pipeline/<pipeline_id>.log`
//!   `<logs_dir>/agent/<agent_id>.log`
//!   `<logs_dir>/worker/<worker_name>.log`

use std::path::{Path, PathBuf};

/// Build the path to a pipeline log file.
///
/// Structure: `{logs_dir}/pipeline/{pipeline_id}.log`
///
/// # Arguments
/// * `logs_dir` - Base logs directory (e.g., `~/.local/state/oj/logs`)
/// * `pipeline_id` - Pipeline identifier
pub fn pipeline_log_path(logs_dir: &Path, pipeline_id: &str) -> PathBuf {
    logs_dir
        .join("pipeline")
        .join(format!("{}.log", pipeline_id))
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

/// Build the path to a pipeline breadcrumb file.
///
/// Structure: `{logs_dir}/{pipeline_id}.crumb.json`
pub fn breadcrumb_path(logs_dir: &Path, pipeline_id: &str) -> PathBuf {
    logs_dir.join(format!("{}.crumb.json", pipeline_id))
}

#[cfg(test)]
#[path = "log_paths_tests.rs"]
mod tests;
