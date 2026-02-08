// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::time::Instant;

use tempfile::tempdir;

use oj_core::{StepOutcome, StepStatus};
use oj_storage::QueueItemStatus;

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
            project_root: temp.path().to_path_buf(),
            namespace: "project-a".to_string(),
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
            assert_eq!(qa.namespace, "project-a");
            assert_eq!(qa.item_count, 1);
            assert_eq!(qa.last_poll_count, None);
            assert_eq!(qa.last_polled_at_ms, None);

            let qb = queues.iter().find(|q| q.name == "builds").unwrap();
            assert_eq!(qb.namespace, "project-b");
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
            oj_storage::QueuePollMeta {
                last_item_count: 5,
                last_polled_at_ms: 1700000000000,
            },
        );
    }

    let response = handle_query(
        Query::ListQueues {
            project_root: temp.path().to_path_buf(),
            namespace: "myproj".to_string(),
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

#[test]
fn get_agent_returns_detail_by_exact_id() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    let agent_id = "pipe123-build";
    {
        let mut s = state.lock();
        let mut p = make_job(
            "pipe123",
            "my-job",
            "myproject",
            "build",
            StepStatus::Running,
            StepOutcome::Running,
            Some(agent_id),
            1000,
        );
        p.workspace_path = Some(std::path::PathBuf::from("/tmp/ws"));
        p.session_id = Some("sess-1".to_string());
        s.jobs.insert("pipe123".to_string(), p);
    }

    let response = handle_query(
        Query::GetAgent {
            agent_id: agent_id.to_string(),
        },
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );

    match response {
        Response::Agent { agent } => {
            let a = agent.expect("agent should be found");
            assert_eq!(a.agent_id, agent_id);
            assert_eq!(a.job_id, "pipe123");
            assert_eq!(a.job_name, "my-job");
            assert_eq!(a.step_name, "build");
            assert_eq!(a.namespace, Some("myproject".to_string()));
            assert_eq!(a.status, "running");
            assert_eq!(a.workspace_path, Some(std::path::PathBuf::from("/tmp/ws")));
            assert_eq!(a.session_id, Some("sess-1".to_string()));
            assert_eq!(a.started_at_ms, 1000);
            assert!(a.error.is_none());
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn get_agent_returns_detail_by_prefix() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.jobs.insert(
            "pipe999".to_string(),
            make_job(
                "pipe999",
                "test-pipe",
                "",
                "deploy",
                StepStatus::Completed,
                StepOutcome::Completed,
                Some("pipe999-deploy"),
                2000,
            ),
        );
    }

    let response = handle_query(
        Query::GetAgent {
            agent_id: "pipe999-dep".to_string(),
        },
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );

    match response {
        Response::Agent { agent } => {
            let a = agent.expect("agent should be found by prefix");
            assert_eq!(a.agent_id, "pipe999-deploy");
            assert_eq!(a.job_name, "test-pipe");
            assert_eq!(a.step_name, "deploy");
            assert_eq!(a.status, "completed");
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
        Query::GetAgent {
            agent_id: "nonexistent".to_string(),
        },
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
fn get_agent_includes_error_for_failed_agent() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.jobs.insert(
            "pipefail".to_string(),
            make_job(
                "pipefail",
                "fail-pipe",
                "proj",
                "check",
                StepStatus::Completed,
                StepOutcome::Failed("compilation error".to_string()),
                Some("pipefail-check"),
                3000,
            ),
        );
    }

    let response = handle_query(
        Query::GetAgent {
            agent_id: "pipefail-check".to_string(),
        },
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );

    match response {
        Response::Agent { agent } => {
            let a = agent.expect("failed agent should be found");
            assert_eq!(a.status, "failed");
            assert_eq!(a.error, Some("compilation error".to_string()));
            assert!(a.exit_reason.as_ref().unwrap().starts_with("failed"));
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
        s.decisions
            .insert("d-old".to_string(), make_decision("d-old", "p1", 1000));
        s.decisions
            .insert("d-mid".to_string(), make_decision("d-mid", "p1", 2000));
        s.decisions
            .insert("d-new".to_string(), make_decision("d-new", "p1", 3000));
    }

    let response = handle_query(
        Query::ListDecisions {
            namespace: "oddjobs".to_string(),
        },
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
