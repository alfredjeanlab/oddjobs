// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::{Pipeline, StepOutcome, StepRecord, StepStatus};
use oj_storage::{MaterializedState, QueueItem, QueueItemStatus, WorkerRecord};

use crate::protocol::{Query, Response};

use super::handle_query;

fn empty_state() -> Arc<Mutex<MaterializedState>> {
    Arc::new(Mutex::new(MaterializedState::default()))
}

fn make_pipeline(
    id: &str,
    name: &str,
    namespace: &str,
    step: &str,
    step_status: StepStatus,
    outcome: StepOutcome,
    agent_id: Option<&str>,
    started_at_ms: u64,
) -> Pipeline {
    Pipeline {
        id: id.to_string(),
        name: name.to_string(),
        kind: "command".to_string(),
        namespace: namespace.to_string(),
        step: step.to_string(),
        step_status,
        step_started_at: Instant::now(),
        step_history: vec![StepRecord {
            name: step.to_string(),
            started_at_ms,
            finished_at_ms: None,
            outcome,
            agent_id: agent_id.map(|s| s.to_string()),
            agent_name: None,
        }],
        vars: HashMap::new(),
        runbook_hash: String::new(),
        cwd: std::path::PathBuf::new(),
        workspace_id: None,
        workspace_path: None,
        session_id: None,
        created_at: Instant::now(),
        error: None,
        action_attempts: HashMap::new(),
        agent_signal: None,
        cancelling: false,
    }
}

fn make_worker(name: &str, namespace: &str, queue: &str, active: usize) -> WorkerRecord {
    WorkerRecord {
        name: name.to_string(),
        namespace: namespace.to_string(),
        project_root: std::path::PathBuf::from("/tmp"),
        runbook_hash: String::new(),
        status: "running".to_string(),
        active_pipeline_ids: (0..active).map(|i| format!("p{}", i)).collect(),
        queue_name: queue.to_string(),
        concurrency: 3,
    }
}

fn make_queue_item(id: &str, status: QueueItemStatus) -> QueueItem {
    QueueItem {
        id: id.to_string(),
        queue_name: "merge".to_string(),
        data: HashMap::new(),
        status,
        worker_name: None,
        pushed_at_epoch_ms: 0,
        failure_count: 0,
    }
}

#[test]
fn status_overview_empty_state() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    let response = handle_query(Query::StatusOverview, &state, temp.path(), start);
    match response {
        Response::StatusOverview {
            uptime_secs: _,
            namespaces,
        } => {
            assert!(namespaces.is_empty());
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn status_overview_groups_by_namespace() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.pipelines.insert(
            "p1".to_string(),
            make_pipeline(
                "p1",
                "fix/login",
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                Some("agent-1"),
                1000,
            ),
        );
        s.pipelines.insert(
            "p2".to_string(),
            make_pipeline(
                "p2",
                "feat/auth",
                "gastown",
                "plan",
                StepStatus::Running,
                StepOutcome::Running,
                None,
                2000,
            ),
        );
    }

    let response = handle_query(Query::StatusOverview, &state, temp.path(), start);
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 2);
            // Sorted alphabetically
            assert_eq!(namespaces[0].namespace, "gastown");
            assert_eq!(namespaces[1].namespace, "oddjobs");

            assert_eq!(namespaces[0].active_pipelines.len(), 1);
            assert_eq!(namespaces[0].active_pipelines[0].name, "feat/auth");

            assert_eq!(namespaces[1].active_pipelines.len(), 1);
            assert_eq!(namespaces[1].active_pipelines[0].name, "fix/login");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn status_overview_separates_escalated() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.pipelines.insert(
            "p1".to_string(),
            make_pipeline(
                "p1",
                "fix/login",
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                None,
                1000,
            ),
        );
        s.pipelines.insert(
            "p2".to_string(),
            make_pipeline(
                "p2",
                "feat/auth",
                "oddjobs",
                "test",
                StepStatus::Waiting,
                StepOutcome::Waiting("gate check failed (exit 1)".to_string()),
                Some("agent-2"),
                2000,
            ),
        );
    }

    let response = handle_query(Query::StatusOverview, &state, temp.path(), start);
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            let ns = &namespaces[0];
            assert_eq!(ns.namespace, "oddjobs");
            assert_eq!(ns.active_pipelines.len(), 1);
            assert_eq!(ns.active_pipelines[0].name, "fix/login");
            assert_eq!(ns.escalated_pipelines.len(), 1);
            assert_eq!(ns.escalated_pipelines[0].name, "feat/auth");
            assert_eq!(
                ns.escalated_pipelines[0].waiting_reason.as_deref(),
                Some("gate check failed (exit 1)")
            );
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn status_overview_excludes_terminal() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        // Terminal pipeline — should be excluded
        s.pipelines.insert(
            "p1".to_string(),
            make_pipeline(
                "p1",
                "fix/done",
                "oddjobs",
                "done",
                StepStatus::Completed,
                StepOutcome::Completed,
                None,
                1000,
            ),
        );
        // Active pipeline — should be included
        s.pipelines.insert(
            "p2".to_string(),
            make_pipeline(
                "p2",
                "fix/active",
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                None,
                2000,
            ),
        );
    }

    let response = handle_query(Query::StatusOverview, &state, temp.path(), start);
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            assert_eq!(namespaces[0].active_pipelines.len(), 1);
            assert_eq!(namespaces[0].active_pipelines[0].name, "fix/active");
            assert!(namespaces[0].escalated_pipelines.is_empty());
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn status_overview_includes_workers_and_queues() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.workers.insert(
            "oddjobs/fix-worker".to_string(),
            make_worker("fix-worker", "oddjobs", "fix", 2),
        );

        s.queue_items.insert(
            "oddjobs/merge".to_string(),
            vec![
                make_queue_item("q1", QueueItemStatus::Pending),
                make_queue_item("q2", QueueItemStatus::Active),
                make_queue_item("q3", QueueItemStatus::Dead),
            ],
        );
    }

    let response = handle_query(Query::StatusOverview, &state, temp.path(), start);
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            let ns = &namespaces[0];
            assert_eq!(ns.namespace, "oddjobs");

            assert_eq!(ns.workers.len(), 1);
            assert_eq!(ns.workers[0].name, "fix-worker");
            assert_eq!(ns.workers[0].active, 2);
            assert_eq!(ns.workers[0].concurrency, 3);

            assert_eq!(ns.queues.len(), 1);
            assert_eq!(ns.queues[0].name, "merge");
            assert_eq!(ns.queues[0].pending, 1);
            assert_eq!(ns.queues[0].active, 1);
            assert_eq!(ns.queues[0].dead, 1);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn status_overview_includes_active_agents() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.pipelines.insert(
            "p1".to_string(),
            make_pipeline(
                "p1",
                "fix/login",
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                Some("claude-abc"),
                1000,
            ),
        );
    }

    let response = handle_query(Query::StatusOverview, &state, temp.path(), start);
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            let ns = &namespaces[0];
            assert_eq!(ns.active_agents.len(), 1);
            assert_eq!(ns.active_agents[0].agent_id, "claude-abc");
            assert_eq!(ns.active_agents[0].pipeline_name, "fix/login");
            assert_eq!(ns.active_agents[0].step_name, "work");
            assert_eq!(ns.active_agents[0].status, "running");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}
