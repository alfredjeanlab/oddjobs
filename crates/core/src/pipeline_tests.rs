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
