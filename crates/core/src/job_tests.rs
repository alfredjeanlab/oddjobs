// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::workspace::WorkspaceId;
use crate::FakeClock;

#[test]
fn job_id_display() {
    let id = JobId::new("test-job");
    assert_eq!(id.to_string(), "test-job");
}

#[test]
fn job_id_equality() {
    let id1 = JobId::new("job-1");
    let id2 = JobId::new("job-1");
    let id3 = JobId::new("job-2");

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
}

#[test]
fn job_id_from_str() {
    let id: JobId = "test".into();
    assert_eq!(id.as_str(), "test");
}

#[test]
fn job_id_serde() {
    let id = JobId::new("my-job");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"my-job\"");

    let parsed: JobId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, id);
}

fn test_config(id: &str) -> JobConfig {
    JobConfig {
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
fn job_creation() {
    let clock = FakeClock::new();
    let config = JobConfig {
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
    let job = Job::new(config, &clock);

    assert_eq!(job.step, "init");
    assert_eq!(job.step_status, StepStatus::Pending);
    assert!(job.workspace_id.is_none());
    assert!(job.workspace_path.is_none());
    assert!(job.session_id.is_none());
}

#[test]
fn job_is_terminal() {
    let clock = FakeClock::new();

    // Not terminal - initial step
    let job = Job::new(test_config("pipe-1"), &clock);
    assert!(!job.is_terminal());

    // Terminal - done
    let mut job = job.clone();
    job.step = "done".to_string();
    assert!(job.is_terminal());

    // Terminal - failed
    let mut job = Job::new(test_config("pipe-1"), &clock);
    job.step = "failed".to_string();
    assert!(job.is_terminal());

    // Terminal - cancelled
    job.step = "cancelled".to_string();
    assert!(job.is_terminal());
}

#[test]
fn job_with_workspace() {
    let clock = FakeClock::new();
    let job = Job::new(test_config("pipe-1"), &clock)
        .with_workspace(WorkspaceId::new("ws-1"), PathBuf::from("/work/space"));

    assert_eq!(job.workspace_id, Some(WorkspaceId::new("ws-1")));
    assert_eq!(job.workspace_path, Some(PathBuf::from("/work/space")));
}

#[test]
fn job_with_session() {
    let clock = FakeClock::new();
    let job = Job::new(test_config("pipe-1"), &clock).with_session("sess-1".to_string());

    assert_eq!(job.session_id, Some("sess-1".to_string()));
    assert_eq!(job.step_status, StepStatus::Running);
}

#[test]
fn job_action_attempts_starts_empty() {
    let clock = FakeClock::new();
    let job = Job::new(test_config("pipe-1"), &clock);
    assert!(job.action_tracker.action_attempts.is_empty());
}

#[test]
fn job_increment_action_attempt() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    // First increment returns 1
    assert_eq!(job.increment_action_attempt("idle", 0), 1);
    // Second increment returns 2
    assert_eq!(job.increment_action_attempt("idle", 0), 2);
    // Third increment returns 3
    assert_eq!(job.increment_action_attempt("idle", 0), 3);
}

#[test]
fn job_get_action_attempt() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    // Unknown key returns 0
    assert_eq!(job.get_action_attempt("unknown", 0), 0);

    // After increment, get returns the count
    job.increment_action_attempt("idle", 0);
    assert_eq!(job.get_action_attempt("idle", 0), 1);

    job.increment_action_attempt("idle", 0);
    assert_eq!(job.get_action_attempt("idle", 0), 2);
}

#[test]
fn job_action_attempts_different_triggers() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    // Different triggers are tracked separately
    assert_eq!(job.increment_action_attempt("idle", 0), 1);
    assert_eq!(job.increment_action_attempt("exit", 0), 1);
    assert_eq!(job.increment_action_attempt("idle", 0), 2);
    assert_eq!(job.increment_action_attempt("exit", 0), 2);

    assert_eq!(job.get_action_attempt("idle", 0), 2);
    assert_eq!(job.get_action_attempt("exit", 0), 2);
}

#[test]
fn job_action_attempts_different_chain_positions() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    // Different chain positions are tracked separately
    assert_eq!(job.increment_action_attempt("idle", 0), 1);
    assert_eq!(job.increment_action_attempt("idle", 1), 1);
    assert_eq!(job.increment_action_attempt("idle", 0), 2);

    assert_eq!(job.get_action_attempt("idle", 0), 2);
    assert_eq!(job.get_action_attempt("idle", 1), 1);
}

#[test]
fn job_reset_action_attempts() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    // Increment some attempts
    job.increment_action_attempt("idle", 0);
    job.increment_action_attempt("idle", 0);
    job.increment_action_attempt("exit", 0);

    assert_eq!(job.get_action_attempt("idle", 0), 2);
    assert_eq!(job.get_action_attempt("exit", 0), 1);

    // Reset clears all attempts
    job.reset_action_attempts();

    assert_eq!(job.get_action_attempt("idle", 0), 0);
    assert_eq!(job.get_action_attempt("exit", 0), 0);
    assert!(job.action_tracker.action_attempts.is_empty());
}

#[test]
fn job_serde_round_trip_with_action_attempts() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    // Populate action_attempts
    job.increment_action_attempt("on_idle", 0);
    job.increment_action_attempt("on_idle", 0);
    job.increment_action_attempt("on_fail", 1);

    // Serialize to JSON (this previously failed with tuple keys)
    let json = serde_json::to_string(&job).expect("serialize job");

    // Deserialize back
    let restored: Job = serde_json::from_str(&json).expect("deserialize job");

    assert_eq!(restored.get_action_attempt("on_idle", 0), 2);
    assert_eq!(restored.get_action_attempt("on_fail", 1), 1);
    assert_eq!(restored.get_action_attempt("unknown", 0), 0);
}

#[test]
fn job_total_retries_starts_zero() {
    let clock = FakeClock::new();
    let job = Job::new(test_config("pipe-1"), &clock);
    assert_eq!(job.total_retries, 0);
}

#[test]
fn job_total_retries_increments_on_retry() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    // First attempt for each trigger does not count as a retry
    job.increment_action_attempt("idle", 0);
    assert_eq!(job.total_retries, 0);

    job.increment_action_attempt("exit", 0);
    assert_eq!(job.total_retries, 0);

    // Second attempt counts as a retry
    job.increment_action_attempt("idle", 0);
    assert_eq!(job.total_retries, 1);

    // Third attempt counts as another retry
    job.increment_action_attempt("idle", 0);
    assert_eq!(job.total_retries, 2);
}

#[test]
fn job_total_retries_persists_across_step_reset() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    // Accumulate some retries
    job.increment_action_attempt("idle", 0);
    job.increment_action_attempt("idle", 0); // retry
    job.increment_action_attempt("idle", 0); // retry
    assert_eq!(job.total_retries, 2);

    // Reset action_attempts (as happens on step transition)
    job.reset_action_attempts();
    assert!(job.action_tracker.action_attempts.is_empty());

    // total_retries is preserved
    assert_eq!(job.total_retries, 2);

    // New step retries continue to accumulate
    job.increment_action_attempt("idle", 0);
    job.increment_action_attempt("idle", 0); // retry
    assert_eq!(job.total_retries, 3);
}

#[test]
fn job_step_visits_starts_empty() {
    let clock = FakeClock::new();
    let job = Job::new(test_config("pipe-1"), &clock);
    assert!(job.step_visits.is_empty());
    assert_eq!(job.get_step_visits("init"), 0);
}

#[test]
fn job_record_step_visit() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    assert_eq!(job.record_step_visit("merge"), 1);
    assert_eq!(job.record_step_visit("merge"), 2);
    assert_eq!(job.record_step_visit("check"), 1);
    assert_eq!(job.record_step_visit("merge"), 3);

    assert_eq!(job.get_step_visits("merge"), 3);
    assert_eq!(job.get_step_visits("check"), 1);
    assert_eq!(job.get_step_visits("unknown"), 0);
}

#[test]
fn job_step_visits_serde_round_trip() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("pipe-1"), &clock);

    job.record_step_visit("merge");
    job.record_step_visit("merge");
    job.record_step_visit("check");

    let json = serde_json::to_string(&job).expect("serialize job");
    let restored: Job = serde_json::from_str(&json).expect("deserialize job");

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
fn step_status_serde_roundtrip() {
    // Waiting with None
    let json = r#"{"Waiting":null}"#;
    let parsed: StepStatus = serde_json::from_str(json).unwrap();
    assert_eq!(parsed, StepStatus::Waiting(None));

    // Waiting with decision_id
    let json_id = r#"{"Waiting":"dec-abc123"}"#;
    let parsed: StepStatus = serde_json::from_str(json_id).unwrap();
    assert_eq!(parsed, StepStatus::Waiting(Some("dec-abc123".to_string())));

    // Roundtrip serialization
    let serialized = serde_json::to_string(&StepStatus::Waiting(None)).unwrap();
    let reparsed: StepStatus = serde_json::from_str(&serialized).unwrap();
    assert_eq!(reparsed, StepStatus::Waiting(None));

    // Unit variants
    assert_eq!(
        serde_json::to_string(&StepStatus::Pending).unwrap(),
        r#""Pending""#
    );
    assert_eq!(
        serde_json::to_string(&StepStatus::Running).unwrap(),
        r#""Running""#
    );
    assert_eq!(
        serde_json::to_string(&StepStatus::Completed).unwrap(),
        r#""Completed""#
    );
    assert_eq!(
        serde_json::to_string(&StepStatus::Failed).unwrap(),
        r#""Failed""#
    );
}
