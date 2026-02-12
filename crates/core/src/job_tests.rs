// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::test_support::strategies::*;
use crate::FakeClock;
use proptest::prelude::*;

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
    JobConfig::builder(id, "build", "init")
        .name("test")
        .runbook_hash("testhash")
        .cwd("/test/project")
        .build()
}

#[test]
fn job_creation() {
    let clock = FakeClock::new();
    let config = JobConfig::builder("job-1", "build", "init")
        .name("test-feature")
        .runbook_hash("testhash")
        .cwd("/test/project")
        .build();
    let job = Job::new(config, &clock);

    assert_eq!(job.step, "init");
    assert_eq!(job.step_status, StepStatus::Pending);
    assert!(job.workspace_id.is_none());
    assert!(job.workspace_path.is_none());
}

#[test]
fn job_is_terminal() {
    let clock = FakeClock::new();

    // Not terminal - initial step
    let job = Job::new(test_config("job-1"), &clock);
    assert!(!job.is_terminal());

    // Terminal - done
    let mut job = job.clone();
    job.step = "done".to_string();
    assert!(job.is_terminal());

    // Terminal - failed
    let mut job = Job::new(test_config("job-1"), &clock);
    job.step = "failed".to_string();
    assert!(job.is_terminal());

    // Terminal - cancelled
    job.step = "cancelled".to_string();
    assert!(job.is_terminal());
}

#[test]
fn job_attempts_starts_empty() {
    let clock = FakeClock::new();
    let job = Job::new(test_config("job-1"), &clock);
    assert!(job.actions.attempts.is_empty());
}

#[test]
fn job_increment_attempt() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

    // First increment returns 1
    assert_eq!(job.increment_attempt("idle", 0), 1);
    // Second increment returns 2
    assert_eq!(job.increment_attempt("idle", 0), 2);
    // Third increment returns 3
    assert_eq!(job.increment_attempt("idle", 0), 3);
}

#[test]
fn job_get_action_attempt() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

    // Unknown key returns 0
    assert_eq!(job.actions.get_action_attempt("unknown", 0), 0);

    // After increment, get returns the count
    job.increment_attempt("idle", 0);
    assert_eq!(job.actions.get_action_attempt("idle", 0), 1);

    job.increment_attempt("idle", 0);
    assert_eq!(job.actions.get_action_attempt("idle", 0), 2);
}

#[test]
fn job_attempts_different_triggers() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

    // Different triggers are tracked separately
    assert_eq!(job.increment_attempt("idle", 0), 1);
    assert_eq!(job.increment_attempt("exit", 0), 1);
    assert_eq!(job.increment_attempt("idle", 0), 2);
    assert_eq!(job.increment_attempt("exit", 0), 2);

    assert_eq!(job.actions.get_action_attempt("idle", 0), 2);
    assert_eq!(job.actions.get_action_attempt("exit", 0), 2);
}

#[test]
fn job_attempts_different_chain_positions() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

    // Different chain positions are tracked separately
    assert_eq!(job.increment_attempt("idle", 0), 1);
    assert_eq!(job.increment_attempt("idle", 1), 1);
    assert_eq!(job.increment_attempt("idle", 0), 2);

    assert_eq!(job.actions.get_action_attempt("idle", 0), 2);
    assert_eq!(job.actions.get_action_attempt("idle", 1), 1);
}

#[test]
fn job_reset_attempts() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

    // Increment some attempts
    job.increment_attempt("idle", 0);
    job.increment_attempt("idle", 0);
    job.increment_attempt("exit", 0);

    assert_eq!(job.actions.get_action_attempt("idle", 0), 2);
    assert_eq!(job.actions.get_action_attempt("exit", 0), 1);

    // Reset clears all attempts
    job.actions.reset_attempts();

    assert_eq!(job.actions.get_action_attempt("idle", 0), 0);
    assert_eq!(job.actions.get_action_attempt("exit", 0), 0);
    assert!(job.actions.attempts.is_empty());
}

#[test]
fn job_serde_round_trip_with_attempts() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

    // Populate attempts
    job.increment_attempt("on_idle", 0);
    job.increment_attempt("on_idle", 0);
    job.increment_attempt("on_fail", 1);

    // Serialize to JSON (this previously failed with tuple keys)
    let json = serde_json::to_string(&job).expect("serialize job");

    // Deserialize back
    let restored: Job = serde_json::from_str(&json).expect("deserialize job");

    assert_eq!(restored.actions.get_action_attempt("on_idle", 0), 2);
    assert_eq!(restored.actions.get_action_attempt("on_fail", 1), 1);
    assert_eq!(restored.actions.get_action_attempt("unknown", 0), 0);
}

#[test]
fn job_total_retries_starts_zero() {
    let clock = FakeClock::new();
    let job = Job::new(test_config("job-1"), &clock);
    assert_eq!(job.total_retries, 0);
}

#[test]
fn job_total_retries_increments_on_retry() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

    // First attempt for each trigger does not count as a retry
    job.increment_attempt("idle", 0);
    assert_eq!(job.total_retries, 0);

    job.increment_attempt("exit", 0);
    assert_eq!(job.total_retries, 0);

    // Second attempt counts as a retry
    job.increment_attempt("idle", 0);
    assert_eq!(job.total_retries, 1);

    // Third attempt counts as another retry
    job.increment_attempt("idle", 0);
    assert_eq!(job.total_retries, 2);
}

#[test]
fn job_total_retries_persists_across_step_reset() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

    // Accumulate some retries
    job.increment_attempt("idle", 0);
    job.increment_attempt("idle", 0); // retry
    job.increment_attempt("idle", 0); // retry
    assert_eq!(job.total_retries, 2);

    // Reset attempts (as happens on step transition)
    job.actions.reset_attempts();
    assert!(job.actions.attempts.is_empty());

    // total_retries is preserved
    assert_eq!(job.total_retries, 2);

    // New step retries continue to accumulate
    job.increment_attempt("idle", 0);
    job.increment_attempt("idle", 0); // retry
    assert_eq!(job.total_retries, 3);
}

#[test]
fn job_step_visits_starts_empty() {
    let clock = FakeClock::new();
    let job = Job::new(test_config("job-1"), &clock);
    assert!(job.step_visits.is_empty());
    assert_eq!(job.get_step_visits("init"), 0);
}

#[test]
fn job_record_step_visit() {
    let clock = FakeClock::new();
    let mut job = Job::new(test_config("job-1"), &clock);

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
    let mut job = Job::new(test_config("job-1"), &clock);

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
    let max = MAX_STEP_VISITS;
    assert!((3..=20).contains(&max), "MAX_STEP_VISITS should be between 3 and 20, got {}", max);
}

#[yare::parameterized(
    pending     = { StepStatus::Pending,                      false },
    running     = { StepStatus::Running,                      false },
    waiting     = { StepStatus::Waiting(None),                true },
    waiting_id  = { StepStatus::Waiting(Some("d-1".into())),  true },
    completed   = { StepStatus::Completed,                    false },
    failed      = { StepStatus::Failed,                       false },
    suspended   = { StepStatus::Suspended,                    false },
)]
fn waiting_iff_waiting_variant(status: StepStatus, expected: bool) {
    assert_eq!(status.is_waiting(), expected);
}

#[yare::parameterized(
    pending     = { StepStatus::Pending,                      false },
    running     = { StepStatus::Running,                      false },
    waiting     = { StepStatus::Waiting(None),                false },
    completed   = { StepStatus::Completed,                    false },
    failed      = { StepStatus::Failed,                       false },
    suspended   = { StepStatus::Suspended,                    true },
)]
fn suspended_iff_suspended_variant(status: StepStatus, expected: bool) {
    assert_eq!(status.is_suspended(), expected);
}

proptest! {
    #[test]
    fn step_status_serde_roundtrip(status in arb_step_status()) {
        let json = serde_json::to_string(&status).unwrap();
        let parsed: StepStatus = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(status, parsed);
    }
}
