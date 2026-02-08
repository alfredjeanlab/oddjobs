// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::time::Instant;

use tempfile::tempdir;

use oj_core::{AgentRun, StepOutcome, StepStatus, StepStatusKind};
use oj_storage::QueueItemStatus;

use super::{
    empty_orphans, empty_state, handle_query, make_breadcrumb, make_job, make_queue_item,
    make_worker, Query, Response,
};

#[test]
fn status_overview_empty_state() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::StatusOverview { namespaces, .. } => {
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
        s.jobs.insert(
            "p1".to_string(),
            make_job(
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
        s.jobs.insert(
            "p2".to_string(),
            make_job(
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

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 2);
            // Sorted alphabetically
            assert_eq!(namespaces[0].namespace, "gastown");
            assert_eq!(namespaces[1].namespace, "oddjobs");

            assert_eq!(namespaces[0].active_jobs.len(), 1);
            assert_eq!(namespaces[0].active_jobs[0].name, "feat/auth");

            assert_eq!(namespaces[1].active_jobs.len(), 1);
            assert_eq!(namespaces[1].active_jobs[0].name, "fix/login");
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
        s.jobs.insert(
            "p1".to_string(),
            make_job(
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
        s.jobs.insert(
            "p2".to_string(),
            make_job(
                "p2",
                "feat/auth",
                "oddjobs",
                "test",
                StepStatus::Waiting(None),
                StepOutcome::Waiting("gate check failed (exit 1)".to_string()),
                Some("agent-2"),
                2000,
            ),
        );
    }

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            let ns = &namespaces[0];
            assert_eq!(ns.namespace, "oddjobs");
            assert_eq!(ns.active_jobs.len(), 1);
            assert_eq!(ns.active_jobs[0].name, "fix/login");
            assert_eq!(ns.escalated_jobs.len(), 1);
            assert_eq!(ns.escalated_jobs[0].name, "feat/auth");
            assert_eq!(
                ns.escalated_jobs[0].waiting_reason.as_deref(),
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
        // Terminal job — should be excluded
        s.jobs.insert(
            "p1".to_string(),
            make_job(
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
        // Active job — should be included
        s.jobs.insert(
            "p2".to_string(),
            make_job(
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

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            assert_eq!(namespaces[0].active_jobs.len(), 1);
            assert_eq!(namespaces[0].active_jobs[0].name, "fix/active");
            assert!(namespaces[0].escalated_jobs.is_empty());
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

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
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

/// Test that jobs and workers in different namespaces both appear in status.
/// This reproduces the bug where a job was running but didn't show up in status
/// because the job was in a different namespace than the workers.
#[test]
fn status_overview_shows_job_in_separate_namespace() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        // Job running in "oddjobs" namespace
        s.jobs.insert(
            "job-1".to_string(),
            make_job(
                "job-1",
                "conflicts-feat-add-runtime-job-1",
                "oddjobs", // Different namespace than workers
                "resolve",
                StepStatus::Running,
                StepOutcome::Running,
                Some("agent-1"),
                1000,
            ),
        );

        // Worker in "wok" namespace (different from job)
        s.workers.insert(
            "wok/merge-conflicts".to_string(),
            make_worker("merge-conflicts", "wok", "merge-conflicts", 0),
        );

        // Queue in "wok" namespace with active item
        s.queue_items.insert(
            "wok/merge-conflicts".to_string(),
            vec![
                make_queue_item("q1", QueueItemStatus::Pending),
                make_queue_item("q2", QueueItemStatus::Active),
            ],
        );
    }

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::StatusOverview { namespaces, .. } => {
            // Both namespaces should appear
            assert_eq!(
                namespaces.len(),
                2,
                "expected both oddjobs and wok namespaces"
            );

            // Sorted alphabetically: oddjobs before wok
            assert_eq!(namespaces[0].namespace, "oddjobs");
            assert_eq!(namespaces[1].namespace, "wok");

            // oddjobs should have the active job
            assert_eq!(namespaces[0].active_jobs.len(), 1);
            assert_eq!(namespaces[0].active_jobs[0].id, "job-1");
            assert_eq!(
                namespaces[0].active_jobs[0].step_status,
                StepStatusKind::Running
            );
            assert!(namespaces[0].workers.is_empty());
            assert!(namespaces[0].queues.is_empty());

            // wok should have worker and queue but no jobs
            assert!(namespaces[1].active_jobs.is_empty());
            assert_eq!(namespaces[1].workers.len(), 1);
            assert_eq!(namespaces[1].workers[0].name, "merge-conflicts");
            assert_eq!(namespaces[1].workers[0].active, 0);
            assert_eq!(namespaces[1].queues.len(), 1);
            assert_eq!(namespaces[1].queues[0].active, 1);
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
        s.agent_runs.insert(
            "ar-1".to_string(),
            AgentRun::builder()
                .id("ar-1")
                .agent_name("coder")
                .command_name("fix/login")
                .namespace("oddjobs")
                .cwd(temp.path())
                .runbook_hash("hash123")
                .agent_id("claude-abc")
                .session_id("tmux-session")
                .build(),
        );
    }

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            let ns = &namespaces[0];
            assert_eq!(ns.active_agents.len(), 1);
            assert_eq!(ns.active_agents[0].agent_id, "claude-abc");
            assert_eq!(ns.active_agents[0].agent_name, "coder");
            assert_eq!(ns.active_agents[0].command_name, "fix/login");
            assert_eq!(ns.active_agents[0].status, "running");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn status_overview_includes_orphans() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();
    let orphans = std::sync::Arc::new(parking_lot::Mutex::new(vec![make_breadcrumb(
        "orphan-status-1",
        "fix/orphan",
        "oddjobs",
        "work",
    )]));

    let response = handle_query(Query::StatusOverview, &state, &orphans, temp.path(), start);
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            let ns = &namespaces[0];
            assert_eq!(ns.namespace, "oddjobs");
            assert_eq!(ns.orphaned_jobs.len(), 1);
            assert_eq!(ns.orphaned_jobs[0].id, "orphan-status-1");
            assert_eq!(ns.orphaned_jobs[0].step_status, StepStatusKind::Orphaned);
            assert!(ns.orphaned_jobs[0].elapsed_ms > 0);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn status_overview_includes_suspended_jobs() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        // Suspended job — should appear in suspended_jobs
        s.jobs.insert(
            "p1".to_string(),
            make_job(
                "p1",
                "fix/paused",
                "oddjobs",
                "suspended",
                StepStatus::Suspended,
                StepOutcome::Failed("user suspended".to_string()),
                Some("agent-1"),
                1000,
            ),
        );
        // Active job — should appear in active_jobs
        s.jobs.insert(
            "p2".to_string(),
            make_job(
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

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::StatusOverview { namespaces, .. } => {
            assert_eq!(namespaces.len(), 1);
            let ns = &namespaces[0];
            assert_eq!(ns.namespace, "oddjobs");
            assert_eq!(ns.active_jobs.len(), 1);
            assert_eq!(ns.active_jobs[0].name, "fix/active");
            assert_eq!(ns.suspended_jobs.len(), 1);
            assert_eq!(ns.suspended_jobs[0].name, "fix/paused");
            assert_eq!(ns.suspended_jobs[0].step_status, StepStatusKind::Suspended);
            assert!(ns.escalated_jobs.is_empty());
        }
        other => panic!("unexpected response: {:?}", other),
    }
}
