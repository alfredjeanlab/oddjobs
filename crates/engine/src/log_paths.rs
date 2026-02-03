// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared path builders for pipeline and agent log files.
//!
//! Used by both the logger (writer) and daemon (reader) to construct
//! consistent paths for logs in the directory structure:
//!   `<logs_dir>/pipeline/<pipeline_id>.log`
//!   `<logs_dir>/agent/<agent_id>.log`

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

/// Build the path to a pipeline breadcrumb file.
///
/// Structure: `{logs_dir}/{pipeline_id}.crumb.json`
pub fn breadcrumb_path(logs_dir: &Path, pipeline_id: &str) -> PathBuf {
    logs_dir.join(format!("{}.crumb.json", pipeline_id))
}

#[cfg(test)]
#[path = "log_paths_tests.rs"]
mod tests;
