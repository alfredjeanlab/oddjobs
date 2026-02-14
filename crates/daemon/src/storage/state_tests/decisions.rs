// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::test_support::crew_created_event;

fn decision_created_event(id: &str, job_id: &str) -> Event {
    Event::DecisionCreated {
        id: DecisionId::from_string(id),
        agent_id: AgentId::from_string("agent-1"),
        owner: JobId::from_string(job_id).into(),
        source: oj_core::DecisionSource::Gate,
        context: "Gate check failed".to_string(),
        options: vec![
            oj_core::DecisionOption::new("Approve").recommended(),
            oj_core::DecisionOption::new("Reject").description("Stop the job"),
        ],
        questions: None,
        created_at_ms: 2_000_000,
        project: "testns".to_string(),
    }
}

fn decision_for_crew(id: &str, run_id: &str, created_at_ms: u64) -> Event {
    Event::DecisionCreated {
        id: DecisionId::from_string(id),
        agent_id: AgentId::from_string("agent-1"),
        owner: CrewId::from_string(run_id).into(),
        source: oj_core::DecisionSource::Idle,
        context: "Agent idle".to_string(),
        options: vec![
            oj_core::DecisionOption::new("Continue"),
            oj_core::DecisionOption::new("Stop"),
        ],
        questions: None,
        created_at_ms,
        project: "testns".to_string(),
    }
}

fn decision_for_job_at(id: &str, job_id: &str, created_at_ms: u64) -> Event {
    Event::DecisionCreated {
        id: DecisionId::from_string(id),
        agent_id: AgentId::from_string("agent-1"),
        owner: JobId::from_string(job_id).into(),
        source: oj_core::DecisionSource::Idle,
        context: "Agent idle".to_string(),
        options: vec![
            oj_core::DecisionOption::new("Continue"),
            oj_core::DecisionOption::new("Stop"),
        ],
        questions: None,
        created_at_ms,
        project: "testns".to_string(),
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
    let state = state_with_job_and_decision("job-1", "dec-abc123");

    let dec = &state.decisions["dec-abc123"];
    assert_eq!(dec.owner, OwnerId::Job(JobId::from_string("job-1")));
    assert_eq!(dec.agent_id, AgentId::from_string("agent-1"));
    assert_eq!(dec.source, oj_core::DecisionSource::Gate);
    assert_eq!(dec.context, "Gate check failed");
    assert_eq!(dec.options.len(), 2);
    assert!(dec.chosen().is_none());
    assert!(dec.resolved_at_ms.is_none());
    assert_eq!(dec.project, "testns");

    let job = &state.jobs["job-1"];
    assert_eq!(job.step_status, oj_core::StepStatus::Waiting(Some("dec-abc123".to_string())));
}

#[test]
fn decision_created_idempotent() {
    let mut state = state_with_job_and_decision("job-1", "dec-abc123");

    state.apply_event(&decision_created_event("dec-abc123", "job-1"));
    assert_eq!(state.decisions.len(), 1);
}

#[test]
fn decision_resolved() {
    let mut state = state_with_job_and_decision("job-1", "dec-abc123");

    state.apply_event(&Event::DecisionResolved {
        id: DecisionId::from_string("dec-abc123"),
        choices: vec![1],
        message: Some("Looks good".to_string()),
        resolved_at_ms: 3_000_000,
        project: "testns".to_string(),
    });

    let dec = &state.decisions["dec-abc123"];
    assert_eq!(dec.chosen(), Some(1));
    assert_eq!(dec.message.as_deref(), Some("Looks good"));
    assert_eq!(dec.resolved_at_ms, Some(3_000_000));
    assert!(dec.is_resolved());
}

#[test]
fn get_decision_prefix_lookup() {
    let state = state_with_job_and_decision("job-1", "dec-abc123");

    assert!(state.get_decision("dec-abc123").is_some());
    assert!(state.get_decision("dec-abc").is_some());
    assert_eq!(state.get_decision("dec-abc").unwrap().id.as_str(), "dec-abc123");
    assert!(state.get_decision("dec-xyz").is_none());
}

// ── Cleanup on job completion / deletion ─────────────────────────────────────

#[yare::parameterized(
    done      = { "done" },
    cancelled = { "cancelled" },
    failed    = { "failed" },
)]
fn job_terminal_removes_unresolved_decisions(terminal_step: &str) {
    let mut state = state_with_job_and_decision("job-1", "dec-1");
    assert!(!state.decisions["dec-1"].is_resolved());

    state.apply_event(&job_transition_event("job-1", terminal_step));

    assert!(!state.decisions.contains_key("dec-1"));
}

#[test]
fn job_terminal_preserves_resolved_decisions() {
    let mut state = state_with_job_and_decision("job-1", "dec-1");

    state.apply_event(&Event::DecisionResolved {
        id: DecisionId::from_string("dec-1"),
        choices: vec![1],
        message: None,
        resolved_at_ms: 3_000_000,
        project: "testns".to_string(),
    });
    assert!(state.decisions["dec-1"].is_resolved());

    state.apply_event(&job_transition_event("job-1", "done"));

    assert!(state.decisions.contains_key("dec-1"));
}

#[test]
fn job_deleted_removes_all_decisions() {
    let mut state = state_with_job_and_decision("job-1", "dec-1");
    state.apply_event(&decision_created_event("dec-2", "job-1"));
    state.apply_event(&Event::DecisionResolved {
        id: DecisionId::from_string("dec-2"),
        choices: vec![1],
        message: None,
        resolved_at_ms: 3_000_000,
        project: "testns".to_string(),
    });

    assert_eq!(state.decisions.len(), 2);

    state.apply_event(&job_delete_event("job-1"));

    assert!(state.decisions.is_empty());
}

#[test]
fn job_deleted_only_removes_own_decisions() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&job_create_event("job-2", "build", "test", "init"));

    state.apply_event(&decision_created_event("dec-1", "job-1"));
    state.apply_event(&decision_created_event("dec-2", "job-2"));

    assert_eq!(state.decisions.len(), 2);

    state.apply_event(&job_delete_event("job-1"));

    assert_eq!(state.decisions.len(), 1);
    assert!(state.decisions.contains_key("dec-2"));
    assert!(!state.decisions.contains_key("dec-1"));
}

#[test]
fn job_terminal_only_removes_own_unresolved_decisions() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&job_create_event("job-2", "build", "test", "init"));

    state.apply_event(&decision_created_event("dec-1", "job-1"));
    state.apply_event(&decision_created_event("dec-2", "job-2"));

    state.apply_event(&job_transition_event("job-1", "done"));

    assert!(!state.decisions.contains_key("dec-1"));
    assert!(state.decisions.contains_key("dec-2"));
}

// ── Auto-supersession ────────────────────────────────────────────────────────

#[test]
fn new_decision_supersedes_previous_for_same_job() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&decision_for_job_at("dec-1", "job-1", 2_000_000));
    assert!(!state.decisions["dec-1"].is_resolved());

    state.apply_event(&decision_for_job_at("dec-2", "job-1", 3_000_000));

    // dec-1 should be auto-dismissed
    let dec1 = &state.decisions["dec-1"];
    assert!(dec1.is_resolved());
    assert_eq!(dec1.resolved_at_ms, Some(3_000_000));
    assert_eq!(dec1.superseded_by.as_ref().unwrap().as_str(), "dec-2");
    assert!(dec1.chosen().is_none());
    assert!(dec1.message.is_none());

    // dec-2 should be the active unresolved decision
    let dec2 = &state.decisions["dec-2"];
    assert!(!dec2.is_resolved());
    assert!(dec2.superseded_by.is_none());
}

#[test]
fn new_decision_supersedes_previous_for_same_crew() {
    let mut state = MaterializedState::default();
    state.apply_event(&crew_created_event("run-1", "worker", "fix"));
    state.apply_event(&decision_for_crew("dec-1", "run-1", 2_000_000));
    assert!(!state.decisions["dec-1"].is_resolved());

    state.apply_event(&decision_for_crew("dec-2", "run-1", 3_000_000));

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
    state.apply_event(&job_create_event("job-1", "build", "test1", "init"));
    state.apply_event(&job_create_event("job-2", "build", "test2", "init"));
    state.apply_event(&decision_for_job_at("dec-1", "job-1", 2_000_000));
    state.apply_event(&decision_for_job_at("dec-2", "job-2", 3_000_000));

    // Neither should be superseded since they have different owners
    assert!(!state.decisions["dec-1"].is_resolved());
    assert!(state.decisions["dec-1"].superseded_by.is_none());
    assert!(!state.decisions["dec-2"].is_resolved());
    assert!(state.decisions["dec-2"].superseded_by.is_none());
}

#[test]
fn new_decision_does_not_affect_already_resolved() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&decision_for_job_at("dec-1", "job-1", 2_000_000));

    // Manually resolve dec-1
    state.apply_event(&Event::DecisionResolved {
        id: DecisionId::from_string("dec-1"),
        choices: vec![1],
        message: Some("approved".to_string()),
        resolved_at_ms: 2_500_000,
        project: "testns".to_string(),
    });
    assert!(state.decisions["dec-1"].is_resolved());

    // Create a new decision for the same job
    state.apply_event(&decision_for_job_at("dec-2", "job-1", 3_000_000));

    // dec-1 should still have its original resolution, not superseded
    let dec1 = &state.decisions["dec-1"];
    assert_eq!(dec1.chosen(), Some(1));
    assert_eq!(dec1.message.as_deref(), Some("approved"));
    assert_eq!(dec1.resolved_at_ms, Some(2_500_000));
    assert!(dec1.superseded_by.is_none());
}

// ── Dominated decisions (less-specific cannot override more-specific) ─────────

#[test]
fn approval_cannot_supersede_question_decision() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));

    // Create a Question decision first
    state.apply_event(&Event::DecisionCreated {
        id: DecisionId::from_string("dec-question"),
        agent_id: AgentId::from_string("agent-1"),
        owner: JobId::from_string("job-1").into(),
        source: oj_core::DecisionSource::Question,
        context: "Which framework?".to_string(),
        options: vec![oj_core::DecisionOption::new("React"), oj_core::DecisionOption::new("Vue")],
        questions: None,
        created_at_ms: 2_000_000,
        project: "testns".to_string(),
    });
    assert!(!state.decisions["dec-question"].is_resolved());

    // Try to create an Approval decision for the same owner
    state.apply_event(&Event::DecisionCreated {
        id: DecisionId::from_string("dec-approval"),
        agent_id: AgentId::from_string("agent-1"),
        owner: JobId::from_string("job-1").into(),
        source: oj_core::DecisionSource::Approval,
        context: "Permission prompt".to_string(),
        options: vec![
            oj_core::DecisionOption::new("Approve"),
            oj_core::DecisionOption::new("Deny"),
        ],
        questions: None,
        created_at_ms: 3_000_000,
        project: "testns".to_string(),
    });

    // Approval decision should NOT be created (dominated by Question)
    assert!(!state.decisions.contains_key("dec-approval"));

    // Question decision should remain unresolved and NOT superseded
    let dec = &state.decisions["dec-question"];
    assert!(!dec.is_resolved());
    assert!(dec.superseded_by.is_none());
}

#[test]
fn question_can_supersede_approval_decision() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));

    // Create an Approval decision first
    state.apply_event(&Event::DecisionCreated {
        id: DecisionId::from_string("dec-approval"),
        agent_id: AgentId::from_string("agent-1"),
        owner: JobId::from_string("job-1").into(),
        source: oj_core::DecisionSource::Approval,
        context: "Permission prompt".to_string(),
        options: vec![
            oj_core::DecisionOption::new("Approve"),
            oj_core::DecisionOption::new("Deny"),
        ],
        questions: None,
        created_at_ms: 2_000_000,
        project: "testns".to_string(),
    });
    assert!(!state.decisions["dec-approval"].is_resolved());

    // Create a Question decision for the same owner
    state.apply_event(&Event::DecisionCreated {
        id: DecisionId::from_string("dec-question"),
        agent_id: AgentId::from_string("agent-1"),
        owner: JobId::from_string("job-1").into(),
        source: oj_core::DecisionSource::Question,
        context: "Which framework?".to_string(),
        options: vec![oj_core::DecisionOption::new("React")],
        questions: None,
        created_at_ms: 3_000_000,
        project: "testns".to_string(),
    });

    // Question decision should be created, superseding the Approval
    assert!(state.decisions.contains_key("dec-question"));
    assert!(!state.decisions["dec-question"].is_resolved());

    // Approval decision should be superseded
    let dec_approval = &state.decisions["dec-approval"];
    assert!(dec_approval.is_resolved());
    assert_eq!(dec_approval.superseded_by.as_ref().unwrap().as_str(), "dec-question");
}

#[test]
fn superseded_decision_cannot_be_resolved() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&decision_for_job_at("dec-1", "job-1", 2_000_000));
    state.apply_event(&decision_for_job_at("dec-2", "job-1", 3_000_000));

    // dec-1 is superseded (resolved)
    assert!(state.decisions["dec-1"].is_resolved());

    // Attempting to resolve it just overwrites the fields (the is_resolved()
    // guard in the daemon prevents this from happening in practice)
    state.apply_event(&Event::DecisionResolved {
        id: DecisionId::from_string("dec-1"),
        choices: vec![2],
        message: None,
        resolved_at_ms: 4_000_000,
        project: "testns".to_string(),
    });

    // The WAL handler always applies, but the superseded_by remains set
    let dec1 = &state.decisions["dec-1"];
    assert!(dec1.is_resolved());
    assert!(dec1.superseded_by.is_some());
}

#[test]
fn decision_resolved_with_no_chosen_auto_dismiss() {
    let mut state = state_with_job_and_decision("job-1", "dec-1");
    assert!(!state.decisions["dec-1"].is_resolved());

    // Simulate auto-dismiss pattern: choices=[], message="auto-dismissed by job resume"
    state.apply_event(&Event::DecisionResolved {
        id: DecisionId::from_string("dec-1"),
        choices: vec![],
        message: Some("auto-dismissed by job resume".to_string()),
        resolved_at_ms: 3_000_000,
        project: "testns".to_string(),
    });

    let dec = &state.decisions["dec-1"];
    assert!(dec.is_resolved());
    assert!(dec.chosen().is_none());
    assert_eq!(dec.message.as_deref(), Some("auto-dismissed by job resume"));
    assert_eq!(dec.resolved_at_ms, Some(3_000_000));
}
