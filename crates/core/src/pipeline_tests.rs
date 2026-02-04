// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::workspace::WorkspaceId;
use crate::FakeClock;

#[test]
fn pipeline_id_display() {
    let id = PipelineId::new("test-pipeline");
    assert_eq!(id.to_string(), "test-pipeline");
}

#[test]
fn pipeline_id_equality() {
    let id1 = PipelineId::new("pipeline-1");
    let id2 = PipelineId::new("pipeline-1");
    let id3 = PipelineId::new("pipeline-2");

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
}

#[test]
fn pipeline_id_from_str() {
    let id: PipelineId = "test".into();
    assert_eq!(id.as_str(), "test");
}

#[test]
fn pipeline_id_serde() {
    let id = PipelineId::new("my-pipeline");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"my-pipeline\"");

    let parsed: PipelineId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, id);
}

fn test_config(id: &str) -> PipelineConfig {
    PipelineConfig {
        id: id.to_string(),
        name: "test".to_string(),
        kind: "build".to_string(),
        vars: HashMap::new(),
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/test/project"),
        initial_step: "init".to_string(),
        namespace: String::new(),
        cron_name: None,
    }
}

#[test]
fn pipeline_creation() {
    let clock = FakeClock::new();
    let config = PipelineConfig {
        id: "pipe-1".to_string(),
        name: "test-feature".to_string(),
        kind: "build".to_string(),
        vars: HashMap::new(),
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/test/project"),
        initial_step: "init".to_string(),
        namespace: String::new(),
        cron_name: None,
    };
    let pipeline = Pipeline::new(config, &clock);

    assert_eq!(pipeline.step, "init");
    assert_eq!(pipeline.step_status, StepStatus::Pending);
    assert!(pipeline.workspace_id.is_none());
    assert!(pipeline.workspace_path.is_none());
    assert!(pipeline.session_id.is_none());
}

#[test]
fn pipeline_is_terminal() {
    let clock = FakeClock::new();

    // Not terminal - initial step
    let pipeline = Pipeline::new(test_config("pipe-1"), &clock);
    assert!(!pipeline.is_terminal());

    // Terminal - done
    let mut pipeline = pipeline.clone();
    pipeline.step = "done".to_string();
    assert!(pipeline.is_terminal());

    // Terminal - failed
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);
    pipeline.step = "failed".to_string();
    assert!(pipeline.is_terminal());

    // Terminal - cancelled
    pipeline.step = "cancelled".to_string();
    assert!(pipeline.is_terminal());
}

#[test]
fn pipeline_with_workspace() {
    let clock = FakeClock::new();
    let pipeline = Pipeline::new(test_config("pipe-1"), &clock)
        .with_workspace(WorkspaceId::new("ws-1"), PathBuf::from("/work/space"));

    assert_eq!(pipeline.workspace_id, Some(WorkspaceId::new("ws-1")));
    assert_eq!(pipeline.workspace_path, Some(PathBuf::from("/work/space")));
}

#[test]
fn pipeline_with_session() {
    let clock = FakeClock::new();
    let pipeline = Pipeline::new(test_config("pipe-1"), &clock).with_session("sess-1".to_string());

    assert_eq!(pipeline.session_id, Some("sess-1".to_string()));
    assert_eq!(pipeline.step_status, StepStatus::Running);
}

#[test]
fn pipeline_action_attempts_starts_empty() {
    let clock = FakeClock::new();
    let pipeline = Pipeline::new(test_config("pipe-1"), &clock);
    assert!(pipeline.action_attempts.is_empty());
}

#[test]
fn pipeline_increment_action_attempt() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    // First increment returns 1
    assert_eq!(pipeline.increment_action_attempt("idle", 0), 1);
    // Second increment returns 2
    assert_eq!(pipeline.increment_action_attempt("idle", 0), 2);
    // Third increment returns 3
    assert_eq!(pipeline.increment_action_attempt("idle", 0), 3);
}

#[test]
fn pipeline_get_action_attempt() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    // Unknown key returns 0
    assert_eq!(pipeline.get_action_attempt("unknown", 0), 0);

    // After increment, get returns the count
    pipeline.increment_action_attempt("idle", 0);
    assert_eq!(pipeline.get_action_attempt("idle", 0), 1);

    pipeline.increment_action_attempt("idle", 0);
    assert_eq!(pipeline.get_action_attempt("idle", 0), 2);
}

#[test]
fn pipeline_action_attempts_different_triggers() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    // Different triggers are tracked separately
    assert_eq!(pipeline.increment_action_attempt("idle", 0), 1);
    assert_eq!(pipeline.increment_action_attempt("exit", 0), 1);
    assert_eq!(pipeline.increment_action_attempt("idle", 0), 2);
    assert_eq!(pipeline.increment_action_attempt("exit", 0), 2);

    assert_eq!(pipeline.get_action_attempt("idle", 0), 2);
    assert_eq!(pipeline.get_action_attempt("exit", 0), 2);
}

#[test]
fn pipeline_action_attempts_different_chain_positions() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    // Different chain positions are tracked separately
    assert_eq!(pipeline.increment_action_attempt("idle", 0), 1);
    assert_eq!(pipeline.increment_action_attempt("idle", 1), 1);
    assert_eq!(pipeline.increment_action_attempt("idle", 0), 2);

    assert_eq!(pipeline.get_action_attempt("idle", 0), 2);
    assert_eq!(pipeline.get_action_attempt("idle", 1), 1);
}

#[test]
fn pipeline_reset_action_attempts() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    // Increment some attempts
    pipeline.increment_action_attempt("idle", 0);
    pipeline.increment_action_attempt("idle", 0);
    pipeline.increment_action_attempt("exit", 0);

    assert_eq!(pipeline.get_action_attempt("idle", 0), 2);
    assert_eq!(pipeline.get_action_attempt("exit", 0), 1);

    // Reset clears all attempts
    pipeline.reset_action_attempts();

    assert_eq!(pipeline.get_action_attempt("idle", 0), 0);
    assert_eq!(pipeline.get_action_attempt("exit", 0), 0);
    assert!(pipeline.action_attempts.is_empty());
}

#[test]
fn pipeline_serde_round_trip_with_action_attempts() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    // Populate action_attempts
    pipeline.increment_action_attempt("on_idle", 0);
    pipeline.increment_action_attempt("on_idle", 0);
    pipeline.increment_action_attempt("on_fail", 1);

    // Serialize to JSON (this previously failed with tuple keys)
    let json = serde_json::to_string(&pipeline).expect("serialize pipeline");

    // Deserialize back
    let restored: Pipeline = serde_json::from_str(&json).expect("deserialize pipeline");

    assert_eq!(restored.get_action_attempt("on_idle", 0), 2);
    assert_eq!(restored.get_action_attempt("on_fail", 1), 1);
    assert_eq!(restored.get_action_attempt("unknown", 0), 0);
}

#[test]
fn pipeline_total_retries_starts_zero() {
    let clock = FakeClock::new();
    let pipeline = Pipeline::new(test_config("pipe-1"), &clock);
    assert_eq!(pipeline.total_retries, 0);
}

#[test]
fn pipeline_total_retries_increments_on_retry() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    // First attempt for each trigger does not count as a retry
    pipeline.increment_action_attempt("idle", 0);
    assert_eq!(pipeline.total_retries, 0);

    pipeline.increment_action_attempt("exit", 0);
    assert_eq!(pipeline.total_retries, 0);

    // Second attempt counts as a retry
    pipeline.increment_action_attempt("idle", 0);
    assert_eq!(pipeline.total_retries, 1);

    // Third attempt counts as another retry
    pipeline.increment_action_attempt("idle", 0);
    assert_eq!(pipeline.total_retries, 2);
}

#[test]
fn pipeline_total_retries_persists_across_step_reset() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    // Accumulate some retries
    pipeline.increment_action_attempt("idle", 0);
    pipeline.increment_action_attempt("idle", 0); // retry
    pipeline.increment_action_attempt("idle", 0); // retry
    assert_eq!(pipeline.total_retries, 2);

    // Reset action_attempts (as happens on step transition)
    pipeline.reset_action_attempts();
    assert!(pipeline.action_attempts.is_empty());

    // total_retries is preserved
    assert_eq!(pipeline.total_retries, 2);

    // New step retries continue to accumulate
    pipeline.increment_action_attempt("idle", 0);
    pipeline.increment_action_attempt("idle", 0); // retry
    assert_eq!(pipeline.total_retries, 3);
}

#[test]
fn pipeline_step_visits_starts_empty() {
    let clock = FakeClock::new();
    let pipeline = Pipeline::new(test_config("pipe-1"), &clock);
    assert!(pipeline.step_visits.is_empty());
    assert_eq!(pipeline.get_step_visits("init"), 0);
}

#[test]
fn pipeline_record_step_visit() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    assert_eq!(pipeline.record_step_visit("merge"), 1);
    assert_eq!(pipeline.record_step_visit("merge"), 2);
    assert_eq!(pipeline.record_step_visit("check"), 1);
    assert_eq!(pipeline.record_step_visit("merge"), 3);

    assert_eq!(pipeline.get_step_visits("merge"), 3);
    assert_eq!(pipeline.get_step_visits("check"), 1);
    assert_eq!(pipeline.get_step_visits("unknown"), 0);
}

#[test]
fn pipeline_step_visits_serde_round_trip() {
    let clock = FakeClock::new();
    let mut pipeline = Pipeline::new(test_config("pipe-1"), &clock);

    pipeline.record_step_visit("merge");
    pipeline.record_step_visit("merge");
    pipeline.record_step_visit("check");

    let json = serde_json::to_string(&pipeline).expect("serialize pipeline");
    let restored: Pipeline = serde_json::from_str(&json).expect("deserialize pipeline");

    assert_eq!(restored.get_step_visits("merge"), 2);
    assert_eq!(restored.get_step_visits("check"), 1);
    assert_eq!(restored.get_step_visits("unknown"), 0);
}

#[test]
fn max_step_visits_is_reasonable() {
    // Sanity check that the constant is a reasonable value
    assert!(
        MAX_STEP_VISITS >= 3 && MAX_STEP_VISITS <= 20,
        "MAX_STEP_VISITS should be between 3 and 20, got {}",
        MAX_STEP_VISITS
    );
}

#[test]
fn step_status_waiting_is_waiting() {
    assert!(StepStatus::Waiting(None).is_waiting());
    assert!(StepStatus::Waiting(Some("dec-1".to_string())).is_waiting());
    assert!(!StepStatus::Pending.is_waiting());
    assert!(!StepStatus::Running.is_waiting());
    assert!(!StepStatus::Completed.is_waiting());
    assert!(!StepStatus::Failed.is_waiting());
}

#[test]
fn step_status_serde_backward_compat() {
    // Old format: unit variant string
    let old_json = r#""Waiting""#;
    let parsed: StepStatus = serde_json::from_str(old_json).unwrap();
    assert_eq!(parsed, StepStatus::Waiting(None));

    // New format: map with null
    let new_json = r#"{"Waiting":null}"#;
    let parsed: StepStatus = serde_json::from_str(new_json).unwrap();
    assert_eq!(parsed, StepStatus::Waiting(None));

    // New format: map with decision_id
    let new_json_id = r#"{"Waiting":"dec-abc123"}"#;
    let parsed: StepStatus = serde_json::from_str(new_json_id).unwrap();
    assert_eq!(parsed, StepStatus::Waiting(Some("dec-abc123".to_string())));

    // Serialization produces the new format
    let serialized = serde_json::to_string(&StepStatus::Waiting(None)).unwrap();
    let reparsed: StepStatus = serde_json::from_str(&serialized).unwrap();
    assert_eq!(reparsed, StepStatus::Waiting(None));
}
