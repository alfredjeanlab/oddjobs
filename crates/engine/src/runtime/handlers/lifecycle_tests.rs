// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for job lifecycle handling (resume)

use crate::test_helpers::{load_runbook_hash, setup_with_runbook, TestContext};
use oj_core::{AgentId, Event, JobId, OwnerId, StepOutcome, StepStatus, WorkspaceId};
use std::collections::HashMap;
use std::path::PathBuf;

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
// handle_workspace_ready: starts first step
// ============================================================================

/// Runbook with a shell step as first step for workspace tests
const SHELL_RUNBOOK: &str = r#"
[job.build]
input = ["prompt"]

[[job.build.step]]
name = "compile"
run = "echo compiling"
on_done = "done"
on_fail = "failed"

[[job.build.step]]
name = "done"
run = "echo done"

[[job.build.step]]
name = "failed"
run = "echo failed"
"#;

/// Create a job with a workspace in Pending step_status (waiting for workspace).
fn create_job_with_workspace(
    ctx: &TestContext,
    job_id: &str,
    runbook_hash: &str,
    workspace_id: &str,
) {
    let workspace_path = ctx.project_root.join("workspaces").join(workspace_id);
    let events = vec![Event::JobCreated {
        id: JobId::new(job_id),
        kind: "build".to_string(),
        name: "test-build".to_string(),
        runbook_hash: runbook_hash.to_string(),
        cwd: workspace_path.clone(),
        vars: HashMap::from([("prompt".to_string(), "Build feature".to_string())]),
        initial_step: "compile".to_string(),
        created_at_epoch_ms: 1_000_000,
        namespace: String::new(),
        cron_name: None,
    }];
    ctx.runtime.lock_state_mut(|state| {
        for event in &events {
            state.apply_event(event);
        }
        // Insert workspace with owner pointing to job
        state.workspaces.insert(
            workspace_id.to_string(),
            oj_storage::Workspace {
                id: workspace_id.to_string(),
                path: workspace_path,
                branch: None,
                owner: Some(OwnerId::Job(JobId::new(job_id))),
                status: oj_core::WorkspaceStatus::Creating,
                workspace_type: oj_storage::WorkspaceType::Folder,
                created_at_ms: 1_000_000,
            },
        );
        // Set workspace_id on job
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.workspace_id = Some(WorkspaceId::new(workspace_id));
            job.workspace_path = Some(state.workspaces[workspace_id].path.clone());
        }
    });
}

#[tokio::test]
async fn workspace_ready_starts_first_step() {
    let ctx = setup_with_runbook(SHELL_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, SHELL_RUNBOOK);
    create_job_with_workspace(&ctx, "job-ws-1", &hash, "ws-test-1");

    // Verify job is in Pending step_status
    let job = ctx
        .runtime
        .lock_state(|s| s.jobs.get("job-ws-1").cloned())
        .unwrap();
    assert_eq!(job.step_status, StepStatus::Pending);
    assert_eq!(job.step, "compile");

    // Fire WorkspaceReady
    let ws_id = WorkspaceId::new("ws-test-1");
    let result = ctx.runtime.handle_workspace_ready(&ws_id).await;

    assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
    let events = result.unwrap();

    // Should contain a StepStarted event for the "compile" step
    let has_step_started = events
        .iter()
        .any(|e| matches!(e, Event::StepStarted { step, .. } if step == "compile"));
    assert!(
        has_step_started,
        "expected StepStarted for 'compile', got: {:?}",
        events
    );
}

#[tokio::test]
async fn workspace_failed_fails_job() {
    let ctx = setup_with_runbook(SHELL_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, SHELL_RUNBOOK);
    create_job_with_workspace(&ctx, "job-ws-2", &hash, "ws-test-2");

    // Fire WorkspaceFailed
    let ws_id = WorkspaceId::new("ws-test-2");
    let result = ctx
        .runtime
        .handle_workspace_failed(&ws_id, "git worktree add failed: no such ref")
        .await;

    assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());

    // Job should be in terminal "failed" state
    let job = ctx
        .runtime
        .lock_state(|s| s.jobs.get("job-ws-2").cloned())
        .unwrap();
    assert_eq!(job.step, "failed", "job should transition to 'failed' step");
}

#[tokio::test]
async fn workspace_ready_idempotent_if_already_running() {
    let ctx = setup_with_runbook(AGENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, AGENT_RUNBOOK);

    // Create a job that is already Running on the "plan" step
    create_running_job(&ctx, "job-ws-3", &hash);

    // Add a workspace record owned by this job
    ctx.runtime.lock_state_mut(|state| {
        state.workspaces.insert(
            "ws-test-3".to_string(),
            oj_storage::Workspace {
                id: "ws-test-3".to_string(),
                path: PathBuf::from("/tmp/ws-test-3"),
                branch: None,
                owner: Some(OwnerId::Job(JobId::new("job-ws-3"))),
                status: oj_core::WorkspaceStatus::Ready,
                workspace_type: oj_storage::WorkspaceType::Folder,
                created_at_ms: 1_000_000,
            },
        );
    });

    // Fire WorkspaceReady on an already-running job → should be a no-op
    let ws_id = WorkspaceId::new("ws-test-3");
    let result = ctx.runtime.handle_workspace_ready(&ws_id).await;

    assert!(result.is_ok());
    let events = result.unwrap();
    assert!(
        events.is_empty(),
        "expected no events for already-running job, got: {:?}",
        events
    );
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
