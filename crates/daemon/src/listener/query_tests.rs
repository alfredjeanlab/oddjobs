// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::{
    AgentRun, AgentRunStatus, Decision, DecisionId, DecisionSource, Job, StepOutcome, StepRecord,
    StepStatus,
};
use oj_storage::{CronRecord, MaterializedState, QueueItem, QueueItemStatus, WorkerRecord};

use oj_engine::breadcrumb::{Breadcrumb, BreadcrumbAgent};

use crate::protocol::{Query, Response};

use super::handle_query;

fn empty_state() -> Arc<Mutex<MaterializedState>> {
    Arc::new(Mutex::new(MaterializedState::default()))
}

fn empty_orphans() -> Arc<Mutex<Vec<Breadcrumb>>> {
    Arc::new(Mutex::new(Vec::new()))
}

fn make_job(
    id: &str,
    name: &str,
    namespace: &str,
    step: &str,
    step_status: StepStatus,
    outcome: StepOutcome,
    agent_id: Option<&str>,
    started_at_ms: u64,
) -> Job {
    Job {
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
        action_tracker: Default::default(),
        cancelling: false,
        total_retries: 0,
        step_visits: HashMap::new(),
        cron_name: None,
        idle_grace_log_size: None,
        last_nudge_at: None,
    }
}

fn make_worker(name: &str, namespace: &str, queue: &str, active: usize) -> WorkerRecord {
    WorkerRecord {
        name: name.to_string(),
        namespace: namespace.to_string(),
        project_root: std::path::PathBuf::from("/tmp"),
        runbook_hash: String::new(),
        status: "running".to_string(),
        active_job_ids: (0..active).map(|i| format!("p{}", i)).collect(),
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

    let response = handle_query(
        Query::StatusOverview,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
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
            assert_eq!(namespaces[0].active_jobs[0].step_status, "running");
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
            AgentRun {
                id: "ar-1".to_string(),
                agent_name: "coder".to_string(),
                command_name: "fix/login".to_string(),
                namespace: "oddjobs".to_string(),
                cwd: temp.path().to_path_buf(),
                runbook_hash: "hash123".to_string(),
                status: AgentRunStatus::Running,
                agent_id: Some("claude-abc".to_string()),
                session_id: Some("tmux-session".to_string()),
                error: None,
                created_at_ms: 1000,
                updated_at_ms: 2000,
                action_tracker: Default::default(),
                vars: HashMap::new(),
                idle_grace_log_size: None,
                last_nudge_at: None,
            },
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

fn make_breadcrumb(job_id: &str, name: &str, project: &str, step: &str) -> Breadcrumb {
    Breadcrumb {
        job_id: job_id.to_string(),
        project: project.to_string(),
        kind: "command".to_string(),
        name: name.to_string(),
        vars: HashMap::new(),
        current_step: step.to_string(),
        step_status: "running".to_string(),
        agents: vec![BreadcrumbAgent {
            agent_id: "orphan-agent-1".to_string(),
            session_name: Some("tmux-orphan-1".to_string()),
            log_path: std::path::PathBuf::from("/tmp/agent.log"),
        }],
        workspace_id: None,
        workspace_root: Some(std::path::PathBuf::from("/tmp/ws")),
        updated_at: "2026-01-15T10:30:00Z".to_string(),
        runbook_hash: "hash123".to_string(),
        cwd: Some(std::path::PathBuf::from("/tmp/project")),
    }
}

#[test]
fn list_jobs_includes_orphans() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();
    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb(
        "orphan-1234",
        "fix/orphan",
        "oddjobs",
        "work",
    )]));

    let response = handle_query(Query::ListJobs, &state, &orphans, temp.path(), start);
    match response {
        Response::Jobs { jobs } => {
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].id, "orphan-1234");
            assert_eq!(jobs[0].name, "fix/orphan");
            assert_eq!(jobs[0].step_status, "orphaned");
            assert_eq!(jobs[0].namespace, "oddjobs");
            assert!(jobs[0].updated_at_ms > 0);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn get_job_falls_back_to_orphan() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();
    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb(
        "orphan-5678",
        "fix/orphan",
        "oddjobs",
        "work",
    )]));

    let response = handle_query(
        Query::GetJob {
            id: "orphan-5678".to_string(),
        },
        &state,
        &orphans,
        temp.path(),
        start,
    );
    match response {
        Response::Job { job } => {
            let p = job.expect("should find orphan job");
            assert_eq!(p.id, "orphan-5678");
            assert_eq!(p.step_status, "orphaned");
            assert_eq!(p.session_id.as_deref(), Some("tmux-orphan-1"));
            assert!(p.error.is_some());
            assert_eq!(p.agents.len(), 1);
            assert_eq!(p.agents[0].status, "orphaned");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn get_job_prefers_state_over_orphan() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    // Add job to both state and orphans with same ID
    {
        let mut s = state.lock();
        s.jobs.insert(
            "shared-id".to_string(),
            make_job(
                "shared-id",
                "fix/real",
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                None,
                1000,
            ),
        );
    }
    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb(
        "shared-id",
        "fix/orphan",
        "oddjobs",
        "work",
    )]));

    let response = handle_query(
        Query::GetJob {
            id: "shared-id".to_string(),
        },
        &state,
        &orphans,
        temp.path(),
        start,
    );
    match response {
        Response::Job { job } => {
            let p = job.expect("should find job");
            // State version should be returned, not orphan
            assert_eq!(p.name, "fix/real");
            assert_ne!(p.step_status, "orphaned");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn status_overview_includes_orphans() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();
    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb(
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
            assert_eq!(ns.orphaned_jobs[0].step_status, "orphaned");
            assert!(ns.orphaned_jobs[0].elapsed_ms > 0);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn get_job_orphan_prefix_match() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();
    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb(
        "orphan-abcdef123456",
        "fix/orphan",
        "oddjobs",
        "work",
    )]));

    // Use prefix to match
    let response = handle_query(
        Query::GetJob {
            id: "orphan-abcdef".to_string(),
        },
        &state,
        &orphans,
        temp.path(),
        start,
    );
    match response {
        Response::Job { job } => {
            let p = job.expect("should find orphan by prefix");
            assert_eq!(p.id, "orphan-abcdef123456");
            assert_eq!(p.step_status, "orphaned");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

fn make_cron(name: &str, namespace: &str, project_root: &str) -> CronRecord {
    CronRecord {
        name: name.to_string(),
        namespace: namespace.to_string(),
        project_root: std::path::PathBuf::from(project_root),
        runbook_hash: String::new(),
        status: "running".to_string(),
        interval: "5m".to_string(),
        run_target: "job:check".to_string(),
        started_at_ms: 0,
        last_fired_at_ms: None,
    }
}

#[test]
fn list_projects_empty_state() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    let response = handle_query(
        Query::ListProjects,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::Projects { projects } => {
            assert!(projects.is_empty());
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_from_workers() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        let mut worker = make_worker("build-worker", "myapp", "build", 1);
        worker.project_root = std::path::PathBuf::from("/home/user/myapp");
        s.workers.insert("myapp/build-worker".to_string(), worker);
    }

    let response = handle_query(
        Query::ListProjects,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::Projects { projects } => {
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].name, "myapp");
            assert_eq!(
                projects[0].root,
                std::path::PathBuf::from("/home/user/myapp")
            );
            assert_eq!(projects[0].workers, 1);
            assert_eq!(projects[0].active_jobs, 0);
            assert_eq!(projects[0].crons, 0);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_from_crons() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.crons.insert(
            "webapp/health".to_string(),
            make_cron("health", "webapp", "/home/user/webapp"),
        );
    }

    let response = handle_query(
        Query::ListProjects,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::Projects { projects } => {
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].name, "webapp");
            assert_eq!(
                projects[0].root,
                std::path::PathBuf::from("/home/user/webapp")
            );
            assert_eq!(projects[0].crons, 1);
            assert_eq!(projects[0].workers, 0);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_from_jobs_with_agents() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.jobs.insert(
            "p1".to_string(),
            make_job(
                "p1",
                "fix/bug",
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                Some("agent-1"),
                1000,
            ),
        );
        // Add a stopped worker so project_root can be resolved
        let mut worker = make_worker("w1", "oddjobs", "q1", 0);
        worker.status = "stopped".to_string();
        worker.project_root = std::path::PathBuf::from("/home/user/oddjobs");
        s.workers.insert("oddjobs/w1".to_string(), worker);
    }

    let response = handle_query(
        Query::ListProjects,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::Projects { projects } => {
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].name, "oddjobs");
            assert_eq!(projects[0].active_jobs, 1);
            assert_eq!(projects[0].active_agents, 1);
            assert_eq!(
                projects[0].root,
                std::path::PathBuf::from("/home/user/oddjobs")
            );
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_multiple_namespaces_sorted() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        let mut w1 = make_worker("w1", "zebra", "q1", 1);
        w1.project_root = std::path::PathBuf::from("/home/user/zebra");
        s.workers.insert("zebra/w1".to_string(), w1);

        let mut w2 = make_worker("w2", "alpha", "q2", 0);
        w2.project_root = std::path::PathBuf::from("/home/user/alpha");
        s.workers.insert("alpha/w2".to_string(), w2);
    }

    let response = handle_query(
        Query::ListProjects,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::Projects { projects } => {
            assert_eq!(projects.len(), 2);
            assert_eq!(projects[0].name, "alpha");
            assert_eq!(projects[1].name, "zebra");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_excludes_stopped_only() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        // Stopped worker with no active jobs or crons
        let mut w = make_worker("w1", "inactive", "q1", 0);
        w.status = "stopped".to_string();
        w.project_root = std::path::PathBuf::from("/home/user/inactive");
        s.workers.insert("inactive/w1".to_string(), w);
    }

    let response = handle_query(
        Query::ListProjects,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::Projects { projects } => {
            assert!(
                projects.is_empty(),
                "stopped-only projects should be excluded"
            );
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_excludes_terminal_jobs() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
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
    }

    let response = handle_query(
        Query::ListProjects,
        &state,
        &empty_orphans(),
        temp.path(),
        start,
    );
    match response {
        Response::Projects { projects } => {
            assert!(
                projects.is_empty(),
                "terminal jobs should not create active projects"
            );
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

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

            let qb = queues.iter().find(|q| q.name == "builds").unwrap();
            assert_eq!(qb.namespace, "project-b");
            assert_eq!(qb.item_count, 2);
            assert_eq!(qb.workers, vec!["worker1".to_string()]);
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
fn get_job_orphan_session_id_from_non_first_agent() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    // Breadcrumb with multiple agents; only the second has session_name set
    let orphans = Arc::new(Mutex::new(vec![Breadcrumb {
        job_id: "orphan-multi-agent".to_string(),
        project: "oddjobs".to_string(),
        kind: "command".to_string(),
        name: "fix/multi".to_string(),
        vars: HashMap::new(),
        current_step: "deploy".to_string(),
        step_status: "running".to_string(),
        agents: vec![
            BreadcrumbAgent {
                agent_id: "agent-plan".to_string(),
                session_name: None,
                log_path: std::path::PathBuf::from("/tmp/agent-plan.log"),
            },
            BreadcrumbAgent {
                agent_id: "agent-deploy".to_string(),
                session_name: Some("tmux-deploy".to_string()),
                log_path: std::path::PathBuf::from("/tmp/agent-deploy.log"),
            },
        ],
        workspace_id: None,
        workspace_root: None,
        updated_at: "2026-01-15T10:30:00Z".to_string(),
        runbook_hash: String::new(),
        cwd: None,
    }]));

    let response = handle_query(
        Query::GetJob {
            id: "orphan-multi-agent".to_string(),
        },
        &state,
        &orphans,
        temp.path(),
        start,
    );
    match response {
        Response::Job { job } => {
            let p = job.expect("should find orphan job");
            assert_eq!(
                p.session_id.as_deref(),
                Some("tmux-deploy"),
                "session_id should come from the agent that has session_name set, not just the first"
            );
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
    std::fs::write(
        log_dir.join("orphan-logs-full-id.log"),
        "line1\nline2\nline3\n",
    )
    .unwrap();

    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb(
        "orphan-logs-full-id",
        "fix/orphan-logs",
        "oddjobs",
        "work",
    )]));

    // Use a prefix to look up logs
    let response = handle_query(
        Query::GetJobLogs {
            id: "orphan-logs".to_string(),
            lines: 0,
        },
        &state,
        &orphans,
        temp.path(),
        start,
    );
    match response {
        Response::JobLogs { log_path, content } => {
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

fn make_decision(id: &str, job_id: &str, created_at_ms: u64) -> Decision {
    Decision {
        id: DecisionId::new(id),
        job_id: job_id.to_string(),
        agent_id: None,
        owner: None,
        source: DecisionSource::Idle,
        context: "test context".to_string(),
        options: vec![],
        chosen: None,
        message: None,
        created_at_ms,
        resolved_at_ms: None,
        namespace: "oddjobs".to_string(),
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
