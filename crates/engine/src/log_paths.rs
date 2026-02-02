// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared path builders for agent log files.
//!
//! Used by both the logger (writer) and daemon (reader) to construct
//! consistent paths for agent logs in the directory structure:
//!   `<logs_dir>/agent/<agent_id>.log`

use std::path::{Path, PathBuf};

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

#[cfg(test)]
#[path = "log_paths_tests.rs"]
mod tests;
