// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

mod agents;
mod attempts;
mod cron;
mod decisions;
mod idempotency;
mod queue;
mod step_history;
mod workers;

use super::*;
pub(super) use oj_core::test_support::{
    agent_spawned_event, job_create_event, job_delete_event, job_transition_event,
    queue_failed_event, queue_pushed_event, queue_taken_event, step_failed_event,
    step_started_event, worker_start_event,
};
use oj_core::{AgentId, CrewId, DecisionId, Event, JobId, OwnerId, StepOutcome, WorkspaceId};

// ── Basic job CRUD ───────────────────────────────────────────────────────────

#[test]
fn apply_event_job_create() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));

    assert!(state.jobs.contains_key("job-1"));
}

#[test]
fn apply_event_job_delete() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&job_delete_event("job-1"));

    assert!(!state.jobs.contains_key("job-1"));
}

#[test]
fn apply_event_job_transition() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));

    assert_eq!(state.jobs["job-1"].step, "init");

    state.apply_event(&job_transition_event("job-1", "build"));

    assert_eq!(state.jobs["job-1"].step, "build");
    assert_eq!(state.jobs["job-1"].step_status, oj_core::StepStatus::Pending);
}

#[test]
fn apply_event_step_started() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));

    state.apply_event(&step_started_event("job-1"));

    assert_eq!(state.jobs["job-1"].step_status, oj_core::StepStatus::Running);
}

#[test]
fn apply_event_step_waiting_with_reason_sets_job_error() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));

    assert!(state.jobs["job-1"].error.is_none());

    state.apply_event(&Event::StepWaiting {
        job_id: JobId::new("job-1"),
        step: "init".to_string(),
        reason: Some("gate `make test` failed (exit 1): tests failed".to_string()),
        decision_id: None,
    });

    assert!(state.jobs["job-1"].step_status.is_waiting());
    assert_eq!(
        state.jobs["job-1"].error.as_deref(),
        Some("gate `make test` failed (exit 1): tests failed")
    );
}

#[test]
fn apply_event_step_started_preserves_existing_error() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));

    state.apply_event(&Event::StepWaiting {
        job_id: JobId::new("job-1"),
        step: "init".to_string(),
        reason: Some("previous error".to_string()),
        decision_id: None,
    });

    // StepStarted should not clear existing error
    state.apply_event(&step_started_event("job-1"));

    assert_eq!(state.jobs["job-1"].error.as_deref(), Some("previous error"));
}

#[test]
fn cancelled_job_is_terminal_after_event_replay() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "execute"));
    state.apply_event(&step_started_event("job-1"));

    state.apply_event(&job_transition_event("job-1", "cancelled"));
    state.apply_event(&step_failed_event("job-1", "execute", "cancelled"));

    let job = &state.jobs["job-1"];
    assert!(job.is_terminal());
    assert_eq!(job.step, "cancelled");
    assert_eq!(job.step_status, oj_core::StepStatus::Failed);
    assert_eq!(job.error.as_deref(), Some("cancelled"));
}

// ── Workspace lifecycle ──────────────────────────────────────────────────────

#[test]
fn apply_event_workspace_lifecycle() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::WorkspaceCreated {
        id: WorkspaceId::new("ws-1"),
        path: PathBuf::from("/tmp/test"),
        branch: Some("feature/test".to_string()),
        owner: JobId::new("job-1").into(),
        workspace_type: None,
    });

    assert!(state.workspaces.contains_key("ws-1"));
    assert_eq!(state.workspaces["ws-1"].path, PathBuf::from("/tmp/test"));
    assert_eq!(state.workspaces["ws-1"].branch, Some("feature/test".to_string()));
    assert_eq!(state.workspaces["ws-1"].owner, JobId::new("job-1"));
    assert_eq!(state.workspaces["ws-1"].status, oj_core::WorkspaceStatus::Creating);

    state.apply_event(&Event::WorkspaceReady { id: WorkspaceId::new("ws-1") });
    assert_eq!(state.workspaces["ws-1"].status, oj_core::WorkspaceStatus::Ready);

    state.apply_event(&Event::WorkspaceDeleted { id: WorkspaceId::new("ws-1") });
    assert!(!state.workspaces.contains_key("ws-1"));
}

#[yare::parameterized(
    folder_explicit   = { Some("folder"),   WorkspaceType::Folder },
    worktree_explicit = { Some("worktree"), WorkspaceType::Worktree },
    none_defaults     = { None,             WorkspaceType::Folder },
)]
fn workspace_type(ws_type: Option<&str>, expected: WorkspaceType) {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::WorkspaceCreated {
        id: WorkspaceId::new("ws-1"),
        path: PathBuf::from("/tmp/ws"),
        branch: None,
        owner: JobId::new("job-1").into(),
        workspace_type: ws_type.map(String::from),
    });

    assert_eq!(state.workspaces["ws-1"].workspace_type, expected);
}

// ── get_job prefix lookup ────────────────────────────────────────────────────

#[test]
fn get_job_exact_match() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-abc123", "build", "test", "init"));

    assert!(state.get_job("job-abc123").is_some());
    assert_eq!(state.get_job("job-abc123").unwrap().id, "job-abc123");
}

#[test]
fn get_job_prefix_match() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-abc123", "build", "test", "init"));

    assert!(state.get_job("job-abc").is_some());
    assert_eq!(state.get_job("job-abc").unwrap().id, "job-abc123");
}

#[test]
fn get_job_ambiguous_prefix() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-abc123", "build", "test1", "init"));
    state.apply_event(&job_create_event("job-abc456", "build", "test2", "init"));

    // "job-abc" matches both, so returns None
    assert!(state.get_job("job-abc").is_none());
}

#[test]
fn get_job_no_match() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-abc123", "build", "test", "init"));

    assert!(state.get_job("job-xyz").is_none());
}

// ── AgentSpawned ─────────────────────────────────────────────────────────────

#[test]
fn apply_event_agent_spawned_is_noop_for_state() {
    let mut state = MaterializedState::default();
    state.apply_event(&agent_spawned_event("agent-1", "job-1"));

    // AgentSpawned is informational — no session tracking in state
    assert!(state.jobs.is_empty());
}
