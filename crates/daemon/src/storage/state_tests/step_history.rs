// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn initialized_on_create() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));

    let job = &state.jobs["job-1"];
    assert_eq!(job.step_history.len(), 1);
    assert_eq!(job.step_history[0].name, "init");
    assert!(job.step_history[0].finished_at_ms.is_none());
    assert_eq!(job.step_history[0].outcome, StepOutcome::Running);
}

#[test]
fn transition_appends_record() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&job_transition_event("job-1", "plan"));

    let job = &state.jobs["job-1"];
    assert_eq!(job.step_history.len(), 2);

    assert_eq!(job.step_history[0].name, "init");
    assert!(job.step_history[0].finished_at_ms.is_some());
    assert_eq!(job.step_history[0].outcome, StepOutcome::Completed);

    assert_eq!(job.step_history[1].name, "plan");
    assert!(job.step_history[1].finished_at_ms.is_none());
    assert_eq!(job.step_history[1].outcome, StepOutcome::Running);
}

#[test]
fn waiting_sets_outcome() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&Event::StepWaiting {
        job_id: JobId::from_string("job-1"),
        step: "init".to_string(),
        reason: Some("gate failed: exit 2".to_string()),
        decision_id: None,
    });

    let job = &state.jobs["job-1"];
    assert_eq!(job.step_history.len(), 1);
    assert!(job.step_history[0].finished_at_ms.is_none());
    assert_eq!(
        job.step_history[0].outcome,
        StepOutcome::Waiting("gate failed: exit 2".to_string())
    );
}

#[test]
fn multi_step_sequence() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&job_transition_event("job-1", "plan"));
    state.apply_event(&job_transition_event("job-1", "implement"));
    state.apply_event(&job_transition_event("job-1", "done"));

    let job = &state.jobs["job-1"];
    assert_eq!(job.step_history.len(), 3); // init, plan, implement (done is terminal)

    assert_eq!(job.step_history[0].name, "init");
    assert_eq!(job.step_history[0].outcome, StepOutcome::Completed);
    assert!(job.step_history[0].finished_at_ms.is_some());

    assert_eq!(job.step_history[1].name, "plan");
    assert_eq!(job.step_history[1].outcome, StepOutcome::Completed);
    assert!(job.step_history[1].finished_at_ms.is_some());

    assert_eq!(job.step_history[2].name, "implement");
    assert_eq!(job.step_history[2].outcome, StepOutcome::Completed);
    assert!(job.step_history[2].finished_at_ms.is_some());
}

#[test]
fn shell_completed_success() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&Event::ShellExited {
        job_id: JobId::from_string("job-1"),
        step: "init".to_string(),
        exit_code: 0,
        stdout: None,
        stderr: None,
    });

    let job = &state.jobs["job-1"];
    assert!(job.step_history[0].finished_at_ms.is_some());
    assert_eq!(job.step_history[0].outcome, StepOutcome::Completed);
}

#[test]
fn shell_completed_failure() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&Event::ShellExited {
        job_id: JobId::from_string("job-1"),
        step: "init".to_string(),
        exit_code: 42,
        stdout: None,
        stderr: None,
    });

    let job = &state.jobs["job-1"];
    assert!(job.step_history[0].finished_at_ms.is_some());
    assert_eq!(job.step_history[0].outcome, StepOutcome::Failed("shell exit code: 42".to_string()));
}

#[test]
fn serde_roundtrip() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&job_transition_event("job-1", "plan"));

    let json = serde_json::to_string(&state).unwrap();
    let restored: MaterializedState = serde_json::from_str(&json).unwrap();

    let job = &restored.jobs["job-1"];
    assert_eq!(job.step_history.len(), 2);
    assert_eq!(job.step_history[0].name, "init");
    assert_eq!(job.step_history[0].outcome, StepOutcome::Completed);
    assert_eq!(job.step_history[1].name, "plan");
    assert_eq!(job.step_history[1].outcome, StepOutcome::Running);
}
