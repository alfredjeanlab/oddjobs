// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use tempfile::tempdir;

use super::{cleanup_agent_files, cleanup_job_files};

#[test]
fn cleanup_job_files_removes_log_and_breadcrumb() {
    let dir = tempdir().unwrap();
    let logs_path = dir.path().join("logs");
    std::fs::create_dir_all(logs_path.join("agent")).unwrap();

    let log_file = oj_core::log_paths::job_log_path(&logs_path, "job-cleanup");
    if let Some(parent) = log_file.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&log_file, "log data").unwrap();

    let crumb_file = oj_core::log_paths::breadcrumb_path(&logs_path, "job-cleanup");
    if let Some(parent) = crumb_file.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&crumb_file, "crumb data").unwrap();

    let agent_log = logs_path.join("agent").join("job-cleanup.log");
    std::fs::write(&agent_log, "agent log").unwrap();

    let agent_dir = logs_path.join("agent").join("job-cleanup");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("session.log"), "session").unwrap();

    cleanup_job_files(&logs_path, "job-cleanup");

    assert!(!log_file.exists(), "job log should be removed");
    assert!(!crumb_file.exists(), "breadcrumb should be removed");
    assert!(!agent_log.exists(), "agent log should be removed");
    assert!(!agent_dir.exists(), "agent dir should be removed");
}

#[test]
fn cleanup_agent_files_removes_log_and_dir() {
    let dir = tempdir().unwrap();
    let logs_path = dir.path().join("logs");
    std::fs::create_dir_all(logs_path.join("agent")).unwrap();

    let agent_log = logs_path.join("agent").join("agent-42.log");
    std::fs::write(&agent_log, "data").unwrap();

    let agent_dir = logs_path.join("agent").join("agent-42");
    std::fs::create_dir_all(&agent_dir).unwrap();

    cleanup_agent_files(&logs_path, "agent-42");

    assert!(!agent_log.exists(), "agent log should be removed");
    assert!(!agent_dir.exists(), "agent dir should be removed");
}
