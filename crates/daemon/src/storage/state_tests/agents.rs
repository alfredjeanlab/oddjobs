// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

fn step_started_with_agent(job_id: &str, agent_id: &str) -> Event {
    Event::StepStarted {
        job_id: JobId::new(job_id),
        step: "init".to_string(),
        agent_id: Some(oj_core::AgentId::new(agent_id)),
        agent_name: Some("worker".to_string()),
    }
}

fn state_with_job_agent(job_id: &str, agent_id: &str) -> MaterializedState {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event(job_id, "build", "test", "init"));
    state.apply_event(&step_started_with_agent(job_id, agent_id));
    state
}

#[test]
fn populated_from_step_started_with_agent() {
    let state = state_with_job_agent("job-1", "agent-abc");

    assert!(state.agents.contains_key("agent-abc"));
    let record = &state.agents["agent-abc"];
    assert_eq!(record.agent_id, "agent-abc");
    assert_eq!(record.agent_name, "worker");
    assert_eq!(record.owner, OwnerId::Job(JobId::new("job-1")));
    assert_eq!(record.status, oj_core::AgentRecordStatus::Starting);
}

#[test]
fn not_populated_from_step_started_without_agent() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&step_started_event("job-1")); // no agent_id

    assert!(state.agents.is_empty());
}

#[test]
fn populated_from_crew_started() {
    let mut state = MaterializedState::default();
    let run_id = CrewId::new("run-1");

    state.apply_event(&Event::CrewCreated {
        id: run_id.clone(),
        agent: "fixer".to_string(),
        command: "fix".to_string(),
        project: "myproj".to_string(),
        cwd: PathBuf::from("/work/dir"),
        runbook_hash: "abc".to_string(),
        vars: HashMap::new(),
        created_at_ms: 1_000,
    });

    state.apply_event(&Event::CrewStarted {
        id: run_id.clone(),
        agent_id: oj_core::AgentId::new("agent-xyz"),
    });

    assert!(state.agents.contains_key("agent-xyz"));
    let record = &state.agents["agent-xyz"];
    assert_eq!(record.agent_name, "fixer");
    assert_eq!(record.owner, OwnerId::Crew(run_id));
    assert_eq!(record.project, "myproj");
    assert_eq!(record.workspace_path, PathBuf::from("/work/dir"));
    assert_eq!(record.status, oj_core::AgentRecordStatus::Running);
}

#[test]
fn status_updates_from_lifecycle_events() {
    let state = state_with_job_agent("job-1", "agent-1");
    assert_eq!(state.agents["agent-1"].status, oj_core::AgentRecordStatus::Starting);

    // Working → Idle → Exited
    let mut state = state;
    state.apply_event(&Event::AgentWorking {
        id: oj_core::AgentId::new("agent-1"),
        owner: JobId::new("job-1").into(),
    });
    assert_eq!(state.agents["agent-1"].status, oj_core::AgentRecordStatus::Running);

    state.apply_event(&Event::AgentWaiting {
        id: oj_core::AgentId::new("agent-1"),
        owner: JobId::new("job-1").into(),
    });
    assert_eq!(state.agents["agent-1"].status, oj_core::AgentRecordStatus::Idle);

    state.apply_event(&Event::AgentExited {
        id: oj_core::AgentId::new("agent-1"),
        exit_code: Some(0),
        owner: JobId::new("job-1").into(),
    });
    assert_eq!(state.agents["agent-1"].status, oj_core::AgentRecordStatus::Exited);
}

#[test]
fn gone_status() {
    let mut state = state_with_job_agent("job-1", "agent-1");

    state.apply_event(&Event::AgentGone {
        id: oj_core::AgentId::new("agent-1"),
        owner: JobId::new("job-1").into(),
        exit_code: None,
    });
    assert_eq!(state.agents["agent-1"].status, oj_core::AgentRecordStatus::Gone);
}

#[test]
fn removed_on_job_deleted() {
    let mut state = state_with_job_agent("job-1", "agent-1");
    assert!(state.agents.contains_key("agent-1"));

    state.apply_event(&job_delete_event("job-1"));

    assert!(!state.agents.contains_key("agent-1"));
}

#[test]
fn removed_on_crew_deleted() {
    let mut state = MaterializedState::default();
    let run_id = CrewId::new("run-1");

    state.apply_event(&Event::CrewCreated {
        id: run_id.clone(),
        agent: "fixer".to_string(),
        command: "fix".to_string(),
        project: "myproj".to_string(),
        cwd: PathBuf::from("/work"),
        runbook_hash: "abc".to_string(),
        vars: HashMap::new(),
        created_at_ms: 1_000,
    });

    state.apply_event(&Event::CrewStarted {
        id: run_id.clone(),
        agent_id: oj_core::AgentId::new("agent-1"),
    });

    assert!(state.agents.contains_key("agent-1"));

    state.apply_event(&Event::CrewDeleted { id: run_id });

    assert!(!state.agents.contains_key("agent-1"));
}

// ── Terminal job guards ──────────────────────────────────────────────────────
// Agent events arriving after a job has already reached a terminal step
// (done/failed/cancelled) must not overwrite the job's status.

#[test]
fn agent_gone_does_not_overwrite_terminal_job() {
    let mut state = state_with_job_agent("job-1", "agent-1");
    // Advance job to terminal "done"
    state.apply_event(&job_transition_event("job-1", "done"));
    assert!(state.jobs["job-1"].is_terminal());
    assert_eq!(state.jobs["job-1"].step_status, oj_core::StepStatus::Completed);

    // Session closes after job already completed
    state.apply_event(&Event::AgentGone {
        id: oj_core::AgentId::new("agent-1"),
        owner: JobId::new("job-1").into(),
        exit_code: None,
    });

    // Job should still be completed, not failed
    assert_eq!(state.jobs["job-1"].step_status, oj_core::StepStatus::Completed);
    assert!(state.jobs["job-1"].error.is_none());
    // Agent record should still update
    assert_eq!(state.agents["agent-1"].status, oj_core::AgentRecordStatus::Gone);
}

#[test]
fn agent_exited_does_not_overwrite_terminal_job() {
    let mut state = state_with_job_agent("job-1", "agent-1");
    state.apply_event(&job_transition_event("job-1", "done"));

    state.apply_event(&Event::AgentExited {
        id: oj_core::AgentId::new("agent-1"),
        exit_code: Some(1),
        owner: JobId::new("job-1").into(),
    });

    assert_eq!(state.jobs["job-1"].step_status, oj_core::StepStatus::Completed);
    assert!(state.jobs["job-1"].error.is_none());
    assert_eq!(state.agents["agent-1"].status, oj_core::AgentRecordStatus::Exited);
}

#[test]
fn agent_failed_does_not_overwrite_terminal_job() {
    let mut state = state_with_job_agent("job-1", "agent-1");
    state.apply_event(&job_transition_event("job-1", "done"));

    state.apply_event(&Event::AgentFailed {
        id: oj_core::AgentId::new("agent-1"),
        error: oj_core::AgentError::Other("api error".to_string()),
        owner: JobId::new("job-1").into(),
    });

    assert_eq!(state.jobs["job-1"].step_status, oj_core::StepStatus::Completed);
    assert!(state.jobs["job-1"].error.is_none());
    assert_eq!(state.agents["agent-1"].status, oj_core::AgentRecordStatus::Exited);
}

// ── Idempotency ─────────────────────────────────────────────────────────────

#[test]
fn idempotent_step_started() {
    let mut state = state_with_job_agent("job-1", "agent-1");

    // Apply again — should not panic or duplicate
    state.apply_event(&step_started_with_agent("job-1", "agent-1"));

    assert_eq!(state.agents.len(), 1);
    assert!(state.agents.contains_key("agent-1"));
}
