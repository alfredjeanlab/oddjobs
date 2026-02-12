// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

/// Current time in milliseconds since epoch.
pub(crate) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Returns `true` if an item is too recent to prune (within the age threshold).
///
/// When `prune_all` is true, always returns `false` (everything is eligible).
pub(crate) fn within_age_threshold(
    prune_all: bool,
    now_ms: u64,
    created_at_ms: u64,
    threshold_ms: u64,
) -> bool {
    !prune_all && created_at_ms > 0 && now_ms.saturating_sub(created_at_ms) < threshold_ms
}

/// Best-effort cleanup of job log, breadcrumb, and associated agent files.
pub(crate) fn cleanup_job_files(logs_path: &std::path::Path, job_id: &str) {
    let log_file = oj_core::log_paths::job_log_path(logs_path, job_id);
    let _ = std::fs::remove_file(&log_file);
    let crumb_file = oj_core::log_paths::breadcrumb_path(logs_path, job_id);
    let _ = std::fs::remove_file(&crumb_file);
    cleanup_agent_files(logs_path, job_id);
}

/// Best-effort cleanup of agent log file and directory.
pub(crate) fn cleanup_agent_files(logs_path: &std::path::Path, agent_id: &str) {
    let agent_log = logs_path.join("agent").join(format!("{}.log", agent_id));
    let _ = std::fs::remove_file(&agent_log);
    let agent_dir = logs_path.join("agent").join(agent_id);
    let _ = std::fs::remove_dir_all(&agent_dir);
}

#[cfg(test)]
#[path = "prune_helpers_tests.rs"]
mod tests;
