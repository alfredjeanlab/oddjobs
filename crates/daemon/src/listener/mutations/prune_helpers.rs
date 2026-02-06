// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

/// Best-effort cleanup of job log, breadcrumb, and associated agent files.
pub(crate) fn cleanup_job_files(logs_path: &std::path::Path, job_id: &str) {
    let log_file = oj_engine::log_paths::job_log_path(logs_path, job_id);
    let _ = std::fs::remove_file(&log_file);
    let crumb_file = oj_engine::log_paths::breadcrumb_path(logs_path, job_id);
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
