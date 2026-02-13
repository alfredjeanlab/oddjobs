// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::time::Instant;

use tempfile::tempdir;

use crate::storage::QueueItemStatus;
use oj_core::{StepOutcome, StepStatus};

use super::{
    empty_orphans, empty_state, handle_query, make_decision, make_job, make_queue_item,
    make_worker, Query, Response,
};

#[test]
fn list_queues_shows_all_namespaces() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    // Add queue items across different namespaces
    {
        let mut s = state.lock();
        s.queue_items.insert(
            "project-a/tasks".to_string(),
            vec![make_queue_item("i1", QueueItemStatus::Pending)],
        );
        s.queue_items.insert(
            "project-b/builds".to_string(),
            vec![
                make_queue_item("i2", QueueItemStatus::Pending),
                make_queue_item("i3", QueueItemStatus::Active),
            ],
        );
        s.workers.insert(
            "project-b/worker1".to_string(),
            make_worker("worker1", "project-b", "builds", 1),
        );
    }

    let response = handle_query(
        Query::ListQueues {
            project_path: temp.path().to_path_buf(),
            project: "project-a".to_string(),
        },
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );

    match response {
        Response::Queues { queues } => {
            assert_eq!(queues.len(), 2, "should show queues from all namespaces");

            let qa = queues.iter().find(|q| q.name == "tasks").unwrap();
            assert_eq!(qa.project, "project-a");
            assert_eq!(qa.item_count, 1);
            assert_eq!(qa.last_poll_count, None);
            assert_eq!(qa.last_polled_at_ms, None);

            let qb = queues.iter().find(|q| q.name == "builds").unwrap();
            assert_eq!(qb.project, "project-b");
            assert_eq!(qb.item_count, 2);
            assert_eq!(qb.workers, vec!["worker1".to_string()]);
            assert_eq!(qb.last_poll_count, None);
            assert_eq!(qb.last_polled_at_ms, None);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_queues_includes_poll_meta() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.queue_items.insert(
            "myproj/tasks".to_string(),
            vec![
                make_queue_item("i1", QueueItemStatus::Pending),
                make_queue_item("i2", QueueItemStatus::Active),
            ],
        );
        s.poll_meta.insert(
            "myproj/tasks".to_string(),
            crate::storage::QueuePollMeta { last_item_count: 5, last_polled_at_ms: 1700000000000 },
        );
    }

    let response = handle_query(
        Query::ListQueues {
            project_path: temp.path().to_path_buf(),
            project: "myproj".to_string(),
        },
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );

    match response {
        Response::Queues { queues } => {
            assert_eq!(queues.len(), 1);
            let q = &queues[0];
            assert_eq!(q.name, "tasks");
            assert_eq!(q.item_count, 2);
            assert_eq!(q.last_poll_count, Some(5));
            assert_eq!(q.last_polled_at_ms, Some(1700000000000));
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[yare::parameterized(
    by_exact_id = {
        "pipe123", "my-job", "myproject", "build",
        StepStatus::Running, StepOutcome::Running,
        "pipe123-build", "pipe123-build", "running", None
    },
    by_prefix = {
        "pipe999", "test-pipe", "", "deploy",
        StepStatus::Completed, StepOutcome::Completed,
        "pipe999-deploy", "pipe999-dep", "completed", None
    },
    failed = {
        "pipefail", "fail-pipe", "proj", "check",
        StepStatus::Completed, StepOutcome::Failed("compilation error".to_string()),
        "pipefail-check", "pipefail-check", "failed", Some("compilation error")
    },
)]
fn get_agent_found(
    job_id: &str,
    job_name: &str,
    project: &str,
    step: &str,
    step_status: StepStatus,
    step_outcome: StepOutcome,
    agent_id: &str,
    query: &str,
    expected_status: &str,
    expected_error: Option<&str>,
) {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.jobs.insert(
            job_id.to_string(),
            make_job(
                job_id,
                job_name,
                project,
                step,
                step_status,
                step_outcome,
                Some(agent_id),
                1000,
            ),
        );
    }

    let response = handle_query(
        Query::GetAgent { agent_id: query.to_string() },
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );

    match response {
        Response::Agent { agent } => {
            let a = agent.expect("agent should be found");
            assert_eq!(a.agent_id, agent_id);
            let owner_job_id = a.owner.as_job().expect("agent should be owned by job");
            assert_eq!(owner_job_id.as_str(), job_id);
            assert_eq!(a.job_name, job_name);
            assert_eq!(a.step_name, step);
            assert_eq!(a.status, expected_status);
            assert_eq!(a.started_at_ms, 1000);
            assert_eq!(a.error.as_deref(), expected_error);
            if !project.is_empty() {
                assert_eq!(a.project, project.to_string());
            }
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn get_agent_returns_none_when_not_found() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    let response = handle_query(
        Query::GetAgent { agent_id: "nonexistent".to_string() },
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );

    match response {
        Response::Agent { agent } => {
            assert!(agent.is_none(), "should return None for unknown agent");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_decisions_returns_most_recent_first() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        // Insert a job so the name can be resolved
        s.jobs.insert(
            "p1".to_string(),
            make_job(
                "p1",
                "fix/bug",
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                None,
                1000,
            ),
        );
        // Insert decisions with different timestamps
        s.decisions.insert("d-old".to_string(), make_decision("d-old", "p1", 1000));
        s.decisions.insert("d-mid".to_string(), make_decision("d-mid", "p1", 2000));
        s.decisions.insert("d-new".to_string(), make_decision("d-new", "p1", 3000));
    }

    let response = handle_query(
        Query::ListDecisions { project: "oddjobs".to_string() },
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::Decisions { decisions } => {
            assert_eq!(decisions.len(), 3);
            // Most recent first
            assert_eq!(decisions[0].id, "d-new");
            assert_eq!(decisions[0].created_at_ms, 3000);
            assert_eq!(decisions[1].id, "d-mid");
            assert_eq!(decisions[1].created_at_ms, 2000);
            assert_eq!(decisions[2].id, "d-old");
            assert_eq!(decisions[2].created_at_ms, 1000);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}
