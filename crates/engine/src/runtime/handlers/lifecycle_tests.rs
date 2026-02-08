// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for job lifecycle handling (resume)

use crate::test_helpers::{load_runbook_hash, setup_with_runbook, TestContext};
use oj_core::{AgentId, Event, JobId, StepOutcome};
use std::collections::HashMap;

/// Runbook with an agent step for testing resume
const AGENT_RUNBOOK: &str = r#"
[job.build]
input = ["prompt"]

[[job.build.step]]
name = "plan"
run = { agent = "planner" }
on_done = "done"
on_fail = "failed"

[[job.build.step]]
name = "done"
run = "echo done"

[[job.build.step]]
name = "failed"
run = "echo failed"

[agent.planner]
run = "claude"
prompt = "${var.prompt}"
"#;

/// Create a job in "failed" state with a step history showing a failed "plan" step.
fn create_failed_job(ctx: &TestContext, job_id: &str, runbook_hash: &str) {
    let events = vec![
        Event::JobCreated {
            id: JobId::new(job_id),
            kind: "build".to_string(),
            name: "test-build".to_string(),
            runbook_hash: runbook_hash.to_string(),
            cwd: ctx.project_root.clone(),
            vars: HashMap::from([("prompt".to_string(), "Build feature".to_string())]),
            initial_step: "plan".to_string(),
            created_at_epoch_ms: 1_000_000,
            namespace: String::new(),
            cron_name: None,
        },
        // Agent started on "plan" step
        Event::StepStarted {
            job_id: JobId::new(job_id),
            step: "plan".to_string(),
            agent_id: Some(AgentId::new("agent-1")),
            agent_name: Some("planner".to_string()),
        },
        // Step failed
        Event::StepFailed {
            job_id: JobId::new(job_id),
            step: "plan".to_string(),
            error: "something went wrong".to_string(),
        },
        // Job transitioned to "failed" terminal state
        Event::JobAdvanced {
            id: JobId::new(job_id),
            step: "failed".to_string(),
        },
    ];
    ctx.runtime.lock_state_mut(|state| {
        for event in &events {
            state.apply_event(event);
        }
    });
}

/// Create a job in running state on agent step "plan".
fn create_running_job(ctx: &TestContext, job_id: &str, runbook_hash: &str) {
    let events = vec![
        Event::JobCreated {
            id: JobId::new(job_id),
            kind: "build".to_string(),
            name: "test-build".to_string(),
            runbook_hash: runbook_hash.to_string(),
            cwd: ctx.project_root.clone(),
            vars: HashMap::from([("prompt".to_string(), "Build feature".to_string())]),
            initial_step: "plan".to_string(),
            created_at_epoch_ms: 1_000_000,
            namespace: String::new(),
            cron_name: None,
        },
        Event::StepStarted {
            job_id: JobId::new(job_id),
            step: "plan".to_string(),
            agent_id: Some(AgentId::new("agent-1")),
            agent_name: Some("planner".to_string()),
        },
    ];
    ctx.runtime.lock_state_mut(|state| {
        for event in &events {
            state.apply_event(event);
        }
    });
}

// ============================================================================
// handle_job_resume: resume from failure with None message
// ============================================================================

#[tokio::test]
async fn resume_failed_job_with_none_message_uses_default() {
    let ctx = setup_with_runbook(AGENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, AGENT_RUNBOOK);
    create_failed_job(&ctx, "job-1", &hash);

    // Verify job is in "failed" state
    let job = ctx.runtime.lock_state(|s| s.jobs.get("job-1").cloned());
    assert_eq!(job.as_ref().unwrap().step, "failed");

    // Resume with no message — should succeed with default "Retrying"
    let job_id = JobId::new("job-1");
    let result = ctx
        .runtime
        .handle_job_resume(&job_id, None, &HashMap::new(), false)
        .await;

    // Should succeed (not error about missing message)
    assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
}

#[tokio::test]
async fn resume_failed_job_returns_job_advanced_event() {
    let ctx = setup_with_runbook(AGENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, AGENT_RUNBOOK);
    create_failed_job(&ctx, "job-1", &hash);

    let job_id = JobId::new("job-1");
    let result = ctx
        .runtime
        .handle_job_resume(&job_id, Some("Try again"), &HashMap::new(), false)
        .await;

    let events = result.unwrap();
    // Should contain a JobAdvanced event (for WAL persistence)
    let has_job_advanced = events.iter().any(|e| {
        matches!(e, Event::JobAdvanced { id, step } if id.as_str() == "job-1" && step == "plan")
    });
    assert!(
        has_job_advanced,
        "expected JobAdvanced event in result for WAL persistence, got: {:?}",
        events
    );
}

// ============================================================================
// handle_job_resume: running job with None message uses default
// ============================================================================

#[tokio::test]
async fn resume_running_agent_job_without_message_uses_default() {
    let ctx = setup_with_runbook(AGENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, AGENT_RUNBOOK);
    create_running_job(&ctx, "job-1", &hash);

    // Verify job is on "plan" step (running)
    let job = ctx.runtime.lock_state(|s| s.jobs.get("job-1").cloned());
    assert_eq!(job.as_ref().unwrap().step, "plan");

    // Resume with no message — should succeed with default
    let job_id = JobId::new("job-1");
    let result = ctx
        .runtime
        .handle_job_resume(&job_id, None, &HashMap::new(), false)
        .await;

    assert!(result.is_ok(), "expected Ok, got: {:?}", result);
}

// ============================================================================
// handle_job_resume: failed job step history has expected outcome
// ============================================================================

#[tokio::test]
async fn failed_job_has_failed_step_in_history() {
    let ctx = setup_with_runbook(AGENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, AGENT_RUNBOOK);
    create_failed_job(&ctx, "job-1", &hash);

    let job = ctx
        .runtime
        .lock_state(|s| s.jobs.get("job-1").cloned())
        .unwrap();
    assert_eq!(job.step, "failed");

    // Verify step history contains a failed "plan" step
    let failed_step = job
        .step_history
        .iter()
        .find(|r| r.name == "plan" && matches!(r.outcome, StepOutcome::Failed(_)));
    assert!(
        failed_step.is_some(),
        "expected a failed 'plan' step in history"
    );
}
