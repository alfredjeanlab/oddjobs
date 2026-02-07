// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn job_log_path_builds_expected_path() {
    let result = job_log_path(Path::new("/state/logs"), "job-001");
    assert_eq!(result, PathBuf::from("/state/logs/job/job-001.log"));
}

#[test]
fn agent_log_path_builds_expected_path() {
    let result = agent_log_path(Path::new("/state/logs"), "abc-123-def");
    assert_eq!(result, PathBuf::from("/state/logs/agent/abc-123-def.log"));
}

#[test]
fn agent_session_log_dir_builds_expected_path() {
    let result = agent_session_log_dir(Path::new("/state/logs"), "abc-123-def");
    assert_eq!(result, PathBuf::from("/state/logs/agent/abc-123-def"));
}

#[test]
fn cron_log_path_builds_expected_path() {
    let result = cron_log_path(Path::new("/state/logs"), "nightly-deploy");
    assert_eq!(result, PathBuf::from("/state/logs/cron/nightly-deploy.log"));
}

#[test]
fn cron_log_path_with_namespace() {
    let result = cron_log_path(Path::new("/state/logs"), "myproject/nightly-deploy");
    assert_eq!(
        result,
        PathBuf::from("/state/logs/cron/myproject/nightly-deploy.log")
    );
}

#[test]
fn worker_log_path_builds_expected_path() {
    let result = worker_log_path(Path::new("/state/logs"), "my-worker");
    assert_eq!(result, PathBuf::from("/state/logs/worker/my-worker.log"));
}

#[test]
fn worker_log_path_with_namespace() {
    let result = worker_log_path(Path::new("/state/logs"), "myproject/my-worker");
    assert_eq!(
        result,
        PathBuf::from("/state/logs/worker/myproject/my-worker.log")
    );
}

#[test]
fn queue_log_path_builds_expected_path() {
    let result = queue_log_path(Path::new("/state/logs"), "build-queue");
    assert_eq!(result, PathBuf::from("/state/logs/queue/build-queue.log"));
}

#[test]
fn queue_log_path_with_namespace() {
    let result = queue_log_path(Path::new("/state/logs"), "myproject/build-queue");
    assert_eq!(
        result,
        PathBuf::from("/state/logs/queue/myproject/build-queue.log")
    );
}

#[test]
fn agent_capture_path_builds_expected_path() {
    let result = agent_capture_path(Path::new("/state/logs"), "abc-123-def");
    assert_eq!(
        result,
        PathBuf::from("/state/logs/agent/abc-123-def/capture.latest.txt")
    );
}

#[test]
fn breadcrumb_path_builds_expected_path() {
    let result = breadcrumb_path(Path::new("/state/logs"), "job-001");
    assert_eq!(result, PathBuf::from("/state/logs/job-001.crumb.json"));
}
