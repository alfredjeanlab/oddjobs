// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[yare::parameterized(
    job_log          = { job_log_path,          "job-001",                   "job/job-001.log" },
    agent_log        = { agent_log_path,        "abc-123-def",               "agent/abc-123-def.log" },
    agent_session    = { agent_session_log_dir, "abc-123-def",               "agent/abc-123-def" },
    cron_log         = { cron_log_path,         "nightly-deploy",            "cron/nightly-deploy.log" },
    cron_namespaced  = { cron_log_path,         "myproject/nightly-deploy",  "cron/myproject/nightly-deploy.log" },
    worker_log       = { worker_log_path,       "my-worker",                 "worker/my-worker.log" },
    worker_namespaced = { worker_log_path,      "myproject/my-worker",       "worker/myproject/my-worker.log" },
    queue_log        = { queue_log_path,        "build-queue",               "queue/build-queue.log" },
    queue_namespaced = { queue_log_path,        "myproject/build-queue",     "queue/myproject/build-queue.log" },
    agent_capture    = { agent_capture_path,    "abc-123-def",               "agent/abc-123-def/capture.latest.txt" },
    breadcrumb       = { breadcrumb_path,       "job-001",                   "job-001.crumb.json" },
)]
fn path_builds_expected(func: fn(&Path, &str) -> PathBuf, id: &str, expected_suffix: &str) {
    assert_eq!(
        func(Path::new("/state/logs"), id),
        PathBuf::from(format!("/state/logs/{}", expected_suffix))
    );
}
