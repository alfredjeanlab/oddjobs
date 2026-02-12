// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::{StepOutcome, StepStatus, StepStatusKind};

use super::{empty_state, handle_query, make_breadcrumb, make_job, Query, Response};

#[test]
fn list_jobs_includes_orphans() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();
    let orphans =
        Arc::new(Mutex::new(vec![make_breadcrumb("orphan-1234", "fix/orphan", "oddjobs", "work")]));

    let response = handle_query(Query::ListJobs, &state, &orphans, temp.path(), start);
    match response {
        Response::Jobs { jobs } => {
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].id, "orphan-1234");
            assert_eq!(jobs[0].name, "fix/orphan");
            assert_eq!(jobs[0].step_status, StepStatusKind::Orphaned);
            assert_eq!(jobs[0].project, "oddjobs");
            assert!(jobs[0].updated_at_ms > 0);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[yare::parameterized(
    falls_back_to_orphan = { "orphan-5678",         "orphan-5678",  None,            "orphan-5678",         true  },
    prefers_state        = { "shared-id",           "shared-id",    Some("fix/real"), "shared-id",           false },
    prefix_match         = { "orphan-abcdef123456", "orphan-abcdef", None,            "orphan-abcdef123456", true  },
)]
fn get_job_orphan_lookup(
    orphan_id: &str,
    query_id: &str,
    state_job_name: Option<&str>,
    expected_id: &str,
    expected_orphaned: bool,
) {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    if let Some(name) = state_job_name {
        let mut s = state.lock();
        s.jobs.insert(
            orphan_id.to_string(),
            make_job(
                orphan_id,
                name,
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                None,
                1000,
            ),
        );
    }

    let orphans =
        Arc::new(Mutex::new(vec![make_breadcrumb(orphan_id, "fix/orphan", "oddjobs", "work")]));

    let response = handle_query(
        Query::GetJob { id: query_id.to_string() },
        &state,
        &orphans,
        temp.path(),
        start,
    );
    match response {
        Response::Job { job } => {
            let p = job.expect("should find job");
            assert_eq!(p.id, expected_id);
            if expected_orphaned {
                assert_eq!(p.step_status, StepStatusKind::Orphaned);
            } else {
                assert_ne!(p.step_status, StepStatusKind::Orphaned);
            }
            if let Some(name) = state_job_name {
                assert_eq!(p.name, name);
            }
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn get_job_logs_resolves_orphan_prefix() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    // Create the job log directory and file
    let log_dir = temp.path().join("job");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("orphan-logs-full-id.log"), "line1\nline2\nline3\n").unwrap();

    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb(
        "orphan-logs-full-id",
        "fix/orphan-logs",
        "oddjobs",
        "work",
    )]));

    // Use a prefix to look up logs
    let response = handle_query(
        Query::GetJobLogs { id: "orphan-logs".to_string(), lines: 0, offset: 0 },
        &state,
        &orphans,
        temp.path(),
        start,
    );
    match response {
        Response::JobLogs { log_path, content, .. } => {
            assert!(
                log_path.ends_with("orphan-logs-full-id.log"),
                "log_path should use the full orphan ID, got: {:?}",
                log_path,
            );
            assert_eq!(content, "line1\nline2\nline3\n");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}
