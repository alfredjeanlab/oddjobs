// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for pipeline step transition effects

use super::*;
use oj_core::{Effect, Pipeline, StepStatus};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

fn test_pipeline() -> Pipeline {
    Pipeline {
        id: "pipe-1".to_string(),
        name: "test-pipeline".to_string(),
        kind: "build".to_string(),
        step: "execute".to_string(),
        step_status: StepStatus::Running,
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/tmp/workspace"),
        session_id: None,
        workspace_id: None,
        workspace_path: None,
        vars: HashMap::new(),
        created_at: Instant::now(),
        step_started_at: Instant::now(),
        error: None,
        step_history: Vec::new(),
        action_attempts: HashMap::new(),
        agent_signal: None,
        namespace: String::new(),
        cancelling: false,
        total_retries: 0,
        step_visits: HashMap::new(),
        cron_name: None,
    }
}

fn has_cancel_timer(effects: &[Effect], timer_id: &str) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::CancelTimer { id } if id == timer_id))
}

fn has_kill_session(effects: &[Effect], expected_id: &str) -> bool {
    effects.iter().any(
        |e| matches!(e, Effect::KillSession { session_id } if session_id.as_str() == expected_id),
    )
}

fn has_session_deleted_event(effects: &[Effect], expected_id: &str) -> bool {
    effects.iter().any(|e| {
        matches!(e, Effect::Emit { event: Event::SessionDeleted { id } } if id.as_str() == expected_id)
    })
}

#[test]
fn completion_effects_cancels_liveness_timer() {
    let pipeline = test_pipeline();
    let effects = completion_effects(&pipeline);
    assert!(
        has_cancel_timer(&effects, "liveness:pipe-1"),
        "completion_effects must cancel the liveness timer"
    );
}

#[test]
fn completion_effects_cancels_exit_deferred_timer() {
    let pipeline = test_pipeline();
    let effects = completion_effects(&pipeline);
    assert!(
        has_cancel_timer(&effects, "exit-deferred:pipe-1"),
        "completion_effects must cancel the exit-deferred timer"
    );
}

#[test]
fn failure_effects_cancels_liveness_timer() {
    let pipeline = test_pipeline();
    let effects = failure_effects(&pipeline, "something went wrong");
    assert!(
        has_cancel_timer(&effects, "liveness:pipe-1"),
        "failure_effects must cancel the liveness timer"
    );
}

#[test]
fn failure_effects_cancels_exit_deferred_timer() {
    let pipeline = test_pipeline();
    let effects = failure_effects(&pipeline, "something went wrong");
    assert!(
        has_cancel_timer(&effects, "exit-deferred:pipe-1"),
        "failure_effects must cancel the exit-deferred timer"
    );
}

#[test]
fn failure_effects_kills_session_when_set() {
    let mut pipeline = test_pipeline();
    pipeline.session_id = Some("sess-agent-1".to_string());
    let effects = failure_effects(&pipeline, "something went wrong");
    assert!(
        has_kill_session(&effects, "sess-agent-1"),
        "failure_effects must kill session when session_id is set"
    );
    assert!(
        has_session_deleted_event(&effects, "sess-agent-1"),
        "failure_effects must emit SessionDeleted when session_id is set"
    );
}

#[test]
fn failure_effects_no_kill_session_when_none() {
    let pipeline = test_pipeline();
    assert!(pipeline.session_id.is_none());
    let effects = failure_effects(&pipeline, "something went wrong");
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::KillSession { .. })),
        "failure_effects must not include KillSession when session_id is None"
    );
}

#[test]
fn completion_effects_kills_session_when_set() {
    let mut pipeline = test_pipeline();
    pipeline.session_id = Some("sess-agent-2".to_string());
    let effects = completion_effects(&pipeline);
    assert!(
        has_kill_session(&effects, "sess-agent-2"),
        "completion_effects must kill session when session_id is set"
    );
    assert!(
        has_session_deleted_event(&effects, "sess-agent-2"),
        "completion_effects must emit SessionDeleted when session_id is set"
    );
}

#[test]
fn cancellation_effects_kills_session_when_set() {
    let mut pipeline = test_pipeline();
    pipeline.session_id = Some("sess-agent-3".to_string());
    let effects = cancellation_effects(&pipeline);
    assert!(
        has_kill_session(&effects, "sess-agent-3"),
        "cancellation_effects must kill session when session_id is set"
    );
    assert!(
        has_session_deleted_event(&effects, "sess-agent-3"),
        "cancellation_effects must emit SessionDeleted when session_id is set"
    );
}

#[test]
fn cancellation_transition_effects_emits_step_failed_and_advance() {
    let pipeline = test_pipeline();
    let effects = cancellation_transition_effects(&pipeline, "cleanup");

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

    // Should emit PipelineAdvanced to the target step
    let has_advanced = effects.iter().any(|e| {
        matches!(
            e,
            Effect::Emit {
                event: Event::PipelineAdvanced { step, .. }
            } if step == "cleanup"
        )
    });
    assert!(
        has_advanced,
        "cancellation_transition_effects must emit PipelineAdvanced to cleanup step"
    );
}

#[test]
fn cancellation_transition_effects_does_not_cancel_timers_or_kill_sessions() {
    let mut pipeline = test_pipeline();
    pipeline.session_id = Some("sess-agent-4".to_string());
    let effects = cancellation_transition_effects(&pipeline, "cleanup");

    // Should NOT cancel timers (runtime handles that separately)
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CancelTimer { .. })),
        "cancellation_transition_effects must not cancel timers"
    );

    // Should NOT kill sessions (runtime handles that separately)
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::KillSession { .. })),
        "cancellation_transition_effects must not kill sessions"
    );
}
