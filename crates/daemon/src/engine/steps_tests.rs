// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for job step transition effects

use super::*;
use oj_core::{Effect, Job};

fn test_job() -> Job {
    Job::builder().id("job-1").cwd("/tmp/workspace").build()
}

fn has_cancel_timer(effects: &[Effect], timer_id: &str) -> bool {
    effects.iter().any(|e| matches!(e, Effect::CancelTimer { id } if id == timer_id))
}

fn has_kill_agent(effects: &[Effect], expected_id: &str) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::KillAgent { agent_id } if agent_id.as_str() == expected_id))
}

fn completion_effects_for(_job: &Job, _err: &str) -> Vec<Effect> {
    completion_effects(_job)
}

#[yare::parameterized(
    completion_cancels_liveness       = { completion_effects_for, "liveness:job:job-1" },
    completion_cancels_exit_deferred  = { completion_effects_for, "exit-deferred:job:job-1" },
    failure_cancels_liveness          = { failure_effects,        "liveness:job:job-1" },
    failure_cancels_exit_deferred     = { failure_effects,        "exit-deferred:job:job-1" },
)]
fn cancels_timer(build_effects: fn(&Job, &str) -> Vec<Effect>, timer_id: &str) {
    let job = test_job();
    let effects = build_effects(&job, "something went wrong");
    assert!(has_cancel_timer(&effects, timer_id), "must cancel timer {}", timer_id);
}

#[test]
fn failure_effects_kills_agent_when_set() {
    let mut job = test_job();
    job.step_history.push(oj_core::StepRecord {
        name: job.step.clone(),
        agent_id: Some("agent-1".to_string()),
        started_at_ms: 0,
        finished_at_ms: None,
        outcome: oj_core::StepOutcome::Running,
        agent_name: None,
    });
    let effects = failure_effects(&job, "something went wrong");
    assert!(
        has_kill_agent(&effects, "agent-1"),
        "failure_effects must kill agent when agent_id is set"
    );
}

#[test]
fn failure_effects_no_kill_agent_when_none() {
    let job = test_job();
    let effects = failure_effects(&job, "something went wrong");
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::KillAgent { .. })),
        "failure_effects must not include KillAgent when agent_id is None"
    );
}

#[test]
fn completion_effects_kills_agent_when_set() {
    let mut job = test_job();
    job.step_history.push(oj_core::StepRecord {
        name: job.step.clone(),
        agent_id: Some("agent-2".to_string()),
        started_at_ms: 0,
        finished_at_ms: None,
        outcome: oj_core::StepOutcome::Running,
        agent_name: None,
    });
    let effects = completion_effects(&job);
    assert!(
        has_kill_agent(&effects, "agent-2"),
        "completion_effects must kill agent when agent_id is set"
    );
}

#[test]
fn cancellation_effects_kills_agent_when_set() {
    let mut job = test_job();
    job.step_history.push(oj_core::StepRecord {
        name: job.step.clone(),
        agent_id: Some("agent-3".to_string()),
        started_at_ms: 0,
        finished_at_ms: None,
        outcome: oj_core::StepOutcome::Running,
        agent_name: None,
    });
    let effects = cancellation_effects(&job);
    assert!(
        has_kill_agent(&effects, "agent-3"),
        "cancellation_effects must kill agent when agent_id is set"
    );
}

#[test]
fn cancellation_transition_effects_emits_step_failed_and_advance() {
    let job = test_job();
    let effects = cancellation_transition_effects(&job, "cleanup");

    // Should emit StepFailed with "cancelled" error
    let has_step_failed = effects.iter().any(|e| {
        matches!(
            e,
            Effect::Emit {
                event: Event::StepFailed { step, error, .. }
            } if step == "execute" && error == "cancelled"
        )
    });
    assert!(
        has_step_failed,
        "cancellation_transition_effects must emit StepFailed with 'cancelled' error"
    );

    // Should emit JobAdvanced to the target step
    let has_advanced = effects.iter().any(|e| {
        matches!(
            e,
            Effect::Emit {
                event: Event::JobAdvanced { step, .. }
            } if step == "cleanup"
        )
    });
    assert!(has_advanced, "cancellation_transition_effects must emit JobAdvanced to cleanup step");
}

#[test]
fn cancellation_transition_effects_does_not_cancel_timers_or_kill_sessions() {
    let job = test_job();
    let effects = cancellation_transition_effects(&job, "cleanup");

    // Should NOT cancel timers (runtime handles that separately)
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::CancelTimer { .. })),
        "cancellation_transition_effects must not cancel timers"
    );

    // Should NOT kill agents (runtime handles that separately)
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::KillAgent { .. })),
        "cancellation_transition_effects must not kill agents"
    );
}
