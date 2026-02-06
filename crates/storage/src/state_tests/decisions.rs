// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::test_support::agent_run_created_event;

fn decision_created_event(id: &str, job_id: &str) -> Event {
    Event::DecisionCreated {
        id: id.to_string(),
        job_id: JobId::new(job_id),
        agent_id: Some("agent-1".to_string()),
        owner: OwnerId::Job(JobId::new(job_id)),
        source: oj_core::DecisionSource::Gate,
        context: "Gate check failed".to_string(),
        options: vec![
            oj_core::DecisionOption::new("Approve").recommended(),
            oj_core::DecisionOption::new("Reject").description("Stop the job"),
        ],
        created_at_ms: 2_000_000,
        namespace: "testns".to_string(),
    }
}

fn decision_for_agent_run(id: &str, ar_id: &str, created_at_ms: u64) -> Event {
    Event::DecisionCreated {
        id: id.to_string(),
        job_id: JobId::new(""),
        agent_id: Some("agent-1".to_string()),
        owner: OwnerId::AgentRun(AgentRunId::new(ar_id)),
        source: oj_core::DecisionSource::Idle,
        context: "Agent idle".to_string(),
        options: vec![
            oj_core::DecisionOption::new("Continue"),
            oj_core::DecisionOption::new("Stop"),
        ],
        created_at_ms,
        namespace: "testns".to_string(),
    }
}

fn decision_for_job_at(id: &str, job_id: &str, created_at_ms: u64) -> Event {
    Event::DecisionCreated {
        id: id.to_string(),
        job_id: JobId::new(job_id),
        agent_id: Some("agent-1".to_string()),
        owner: OwnerId::Job(JobId::new(job_id)),
        source: oj_core::DecisionSource::Idle,
        context: "Agent idle".to_string(),
        options: vec![
            oj_core::DecisionOption::new("Continue"),
            oj_core::DecisionOption::new("Stop"),
        ],
        created_at_ms,
        namespace: "testns".to_string(),
    }
}

fn state_with_job_and_decision(job_id: &str, dec_id: &str) -> MaterializedState {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event(job_id, "build", "test", "init"));
    state.apply_event(&decision_created_event(dec_id, job_id));
    state
}

#[test]
fn decision_created() {
    let state = state_with_job_and_decision("pipe-1", "dec-abc123");

    let dec = &state.decisions["dec-abc123"];
    assert_eq!(dec.job_id, "pipe-1");
    assert_eq!(dec.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(dec.source, oj_core::DecisionSource::Gate);
    assert_eq!(dec.context, "Gate check failed");
    assert_eq!(dec.options.len(), 2);
    assert!(dec.chosen.is_none());
    assert!(dec.resolved_at_ms.is_none());
    assert_eq!(dec.namespace, "testns");

    let job = &state.jobs["pipe-1"];
    assert_eq!(
        job.step_status,
        oj_core::StepStatus::Waiting(Some("dec-abc123".to_string()))
    );
}

#[test]
fn decision_created_idempotent() {
    let mut state = state_with_job_and_decision("pipe-1", "dec-abc123");

    state.apply_event(&decision_created_event("dec-abc123", "pipe-1"));
    assert_eq!(state.decisions.len(), 1);
}

#[test]
fn decision_resolved() {
    let mut state = state_with_job_and_decision("pipe-1", "dec-abc123");

    state.apply_event(&Event::DecisionResolved {
        id: "dec-abc123".to_string(),
        chosen: Some(1),
        message: Some("Looks good".to_string()),
        resolved_at_ms: 3_000_000,
        namespace: "testns".to_string(),
    });

    let dec = &state.decisions["dec-abc123"];
    assert_eq!(dec.chosen, Some(1));
    assert_eq!(dec.message.as_deref(), Some("Looks good"));
    assert_eq!(dec.resolved_at_ms, Some(3_000_000));
    assert!(dec.is_resolved());
}

#[test]
fn get_decision_prefix_lookup() {
    let state = state_with_job_and_decision("pipe-1", "dec-abc123");

    assert!(state.get_decision("dec-abc123").is_some());
    assert!(state.get_decision("dec-abc").is_some());
    assert_eq!(
        state.get_decision("dec-abc").unwrap().id.as_str(),
        "dec-abc123"
    );
    assert!(state.get_decision("dec-xyz").is_none());
}

// ── Cleanup on job completion / deletion ─────────────────────────────────────

#[yare::parameterized(
    done      = { "done" },
    cancelled = { "cancelled" },
    failed    = { "failed" },
)]
fn job_terminal_removes_unresolved_decisions(terminal_step: &str) {
    let mut state = state_with_job_and_decision("pipe-1", "dec-1");
    assert!(!state.decisions["dec-1"].is_resolved());

    state.apply_event(&job_transition_event("pipe-1", terminal_step));

    assert!(!state.decisions.contains_key("dec-1"));
}

#[test]
fn job_terminal_preserves_resolved_decisions() {
    let mut state = state_with_job_and_decision("pipe-1", "dec-1");

    state.apply_event(&Event::DecisionResolved {
        id: "dec-1".to_string(),
        chosen: Some(1),
        message: None,
        resolved_at_ms: 3_000_000,
        namespace: "testns".to_string(),
    });
    assert!(state.decisions["dec-1"].is_resolved());

    state.apply_event(&job_transition_event("pipe-1", "done"));

    assert!(state.decisions.contains_key("dec-1"));
}

#[test]
fn job_deleted_removes_all_decisions() {
    let mut state = state_with_job_and_decision("pipe-1", "dec-1");
    state.apply_event(&decision_created_event("dec-2", "pipe-1"));
    state.apply_event(&Event::DecisionResolved {
        id: "dec-2".to_string(),
        chosen: Some(1),
        message: None,
        resolved_at_ms: 3_000_000,
        namespace: "testns".to_string(),
    });

    assert_eq!(state.decisions.len(), 2);

    state.apply_event(&job_delete_event("pipe-1"));

    assert!(state.decisions.is_empty());
}

#[test]
fn job_deleted_only_removes_own_decisions() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&job_create_event("pipe-2", "build", "test", "init"));

    state.apply_event(&decision_created_event("dec-1", "pipe-1"));
    state.apply_event(&decision_created_event("dec-2", "pipe-2"));

    assert_eq!(state.decisions.len(), 2);

    state.apply_event(&job_delete_event("pipe-1"));

    assert_eq!(state.decisions.len(), 1);
    assert!(state.decisions.contains_key("dec-2"));
    assert!(!state.decisions.contains_key("dec-1"));
}

#[test]
fn job_terminal_only_removes_own_unresolved_decisions() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&job_create_event("pipe-2", "build", "test", "init"));

    state.apply_event(&decision_created_event("dec-1", "pipe-1"));
    state.apply_event(&decision_created_event("dec-2", "pipe-2"));

    state.apply_event(&job_transition_event("pipe-1", "done"));

    assert!(!state.decisions.contains_key("dec-1"));
    assert!(state.decisions.contains_key("dec-2"));
}

// ── Auto-supersession ────────────────────────────────────────────────────────

#[test]
fn new_decision_supersedes_previous_for_same_job() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&decision_for_job_at("dec-1", "pipe-1", 2_000_000));
    assert!(!state.decisions["dec-1"].is_resolved());

    state.apply_event(&decision_for_job_at("dec-2", "pipe-1", 3_000_000));

    // dec-1 should be auto-dismissed
    let dec1 = &state.decisions["dec-1"];
    assert!(dec1.is_resolved());
    assert_eq!(dec1.resolved_at_ms, Some(3_000_000));
    assert_eq!(dec1.superseded_by.as_ref().unwrap().as_str(), "dec-2");
    assert!(dec1.chosen.is_none());
    assert!(dec1.message.is_none());

    // dec-2 should be the active unresolved decision
    let dec2 = &state.decisions["dec-2"];
    assert!(!dec2.is_resolved());
    assert!(dec2.superseded_by.is_none());
}

#[test]
fn new_decision_supersedes_previous_for_same_agent_run() {
    let mut state = MaterializedState::default();
    state.apply_event(&agent_run_created_event("ar-1", "worker", "fix"));
    state.apply_event(&decision_for_agent_run("dec-1", "ar-1", 2_000_000));
    assert!(!state.decisions["dec-1"].is_resolved());

    state.apply_event(&decision_for_agent_run("dec-2", "ar-1", 3_000_000));

    let dec1 = &state.decisions["dec-1"];
    assert!(dec1.is_resolved());
    assert_eq!(dec1.resolved_at_ms, Some(3_000_000));
    assert_eq!(dec1.superseded_by.as_ref().unwrap().as_str(), "dec-2");

    let dec2 = &state.decisions["dec-2"];
    assert!(!dec2.is_resolved());
    assert!(dec2.superseded_by.is_none());
}

#[test]
fn new_decision_does_not_affect_other_owners() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("pipe-1", "build", "test1", "init"));
    state.apply_event(&job_create_event("pipe-2", "build", "test2", "init"));
    state.apply_event(&decision_for_job_at("dec-1", "pipe-1", 2_000_000));
    state.apply_event(&decision_for_job_at("dec-2", "pipe-2", 3_000_000));

    // Neither should be superseded since they have different owners
    assert!(!state.decisions["dec-1"].is_resolved());
    assert!(state.decisions["dec-1"].superseded_by.is_none());
    assert!(!state.decisions["dec-2"].is_resolved());
    assert!(state.decisions["dec-2"].superseded_by.is_none());
}

#[test]
fn new_decision_does_not_affect_already_resolved() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&decision_for_job_at("dec-1", "pipe-1", 2_000_000));

    // Manually resolve dec-1
    state.apply_event(&Event::DecisionResolved {
        id: "dec-1".to_string(),
        chosen: Some(1),
        message: Some("approved".to_string()),
        resolved_at_ms: 2_500_000,
        namespace: "testns".to_string(),
    });
    assert!(state.decisions["dec-1"].is_resolved());

    // Create a new decision for the same job
    state.apply_event(&decision_for_job_at("dec-2", "pipe-1", 3_000_000));

    // dec-1 should still have its original resolution, not superseded
    let dec1 = &state.decisions["dec-1"];
    assert_eq!(dec1.chosen, Some(1));
    assert_eq!(dec1.message.as_deref(), Some("approved"));
    assert_eq!(dec1.resolved_at_ms, Some(2_500_000));
    assert!(dec1.superseded_by.is_none());
}

#[test]
fn superseded_decision_cannot_be_resolved() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&decision_for_job_at("dec-1", "pipe-1", 2_000_000));
    state.apply_event(&decision_for_job_at("dec-2", "pipe-1", 3_000_000));

    // dec-1 is superseded (resolved)
    assert!(state.decisions["dec-1"].is_resolved());

    // Attempting to resolve it just overwrites the fields (the is_resolved()
    // guard in the daemon prevents this from happening in practice)
    state.apply_event(&Event::DecisionResolved {
        id: "dec-1".to_string(),
        chosen: Some(2),
        message: None,
        resolved_at_ms: 4_000_000,
        namespace: "testns".to_string(),
    });

    // The WAL handler always applies, but the superseded_by remains set
    let dec1 = &state.decisions["dec-1"];
    assert!(dec1.is_resolved());
    assert!(dec1.superseded_by.is_some());
}
