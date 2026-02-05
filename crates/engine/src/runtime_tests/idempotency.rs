// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for idempotency guards in job and agent creation.
//!
//! These tests verify that handlers properly skip duplicate creation
//! when events are re-processed during crash recovery.

use super::*;

const SHELL_RUNBOOK: &str = r#"
[command.shell-test]
run = "echo hello"
"#;

const AGENT_RUNBOOK: &str = r#"
[command.agent-test]
run = { agent = "test-agent" }

[agent.test-agent]
run = "claude --print"
"#;

const JOB_RUNBOOK: &str = r#"
[command.job-test]
run = { job = "test-job" }

[job.test-job]
[[job.test-job.step]]
name = "work"
run = "echo working"
"#;

/// Test that create_and_start_job is idempotent - processing the same
/// job creation event twice should not create duplicate jobs or fail.
#[tokio::test]
async fn create_and_start_job_is_idempotent() {
    let ctx = setup_with_runbook(JOB_RUNBOOK).await;

    let args: HashMap<String, String> = HashMap::new();
    let event = command_event(
        "pipe-1",
        "test-job",
        "job-test",
        args.clone(),
        &ctx.project_root,
    );

    // First call: creates the job
    let events1 = ctx.runtime.handle_event(event.clone()).await.unwrap();
    assert!(!events1.is_empty(), "first call should produce events");

    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1, "should have exactly one job");

    // Second call: should be a no-op due to idempotency guard
    let events2 = ctx.runtime.handle_event(event).await.unwrap();
    assert!(
        events2.is_empty(),
        "second call should produce no events (idempotent)"
    );

    // Still exactly one job
    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1, "should still have exactly one job");
}

/// Test that shell command handler is idempotent.
#[tokio::test]
async fn shell_command_is_idempotent() {
    let ctx = setup_with_runbook(SHELL_RUNBOOK).await;

    let args: HashMap<String, String> = HashMap::new();
    let event = command_event(
        "shell-1",
        "shell-test",
        "shell-test",
        args.clone(),
        &ctx.project_root,
    );

    // First call: creates the job and runs shell
    let events1 = ctx.runtime.handle_event(event.clone()).await.unwrap();
    assert!(!events1.is_empty(), "first call should produce events");

    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1, "should have exactly one job");

    // Second call: should be a no-op due to idempotency guard
    let events2 = ctx.runtime.handle_event(event).await.unwrap();
    assert!(
        events2.is_empty(),
        "second call should produce no events (idempotent)"
    );

    // Still exactly one job
    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1, "should still have exactly one job");
}

/// Test that standalone agent command handler is idempotent.
#[tokio::test]
async fn standalone_agent_command_is_idempotent() {
    let ctx = setup_with_runbook(AGENT_RUNBOOK).await;

    let args: HashMap<String, String> = HashMap::new();
    let event = command_event(
        "agent-1",
        "agent-test",
        "agent-test",
        args.clone(),
        &ctx.project_root,
    );

    // First call: creates the agent run
    let events1 = ctx.runtime.handle_event(event.clone()).await.unwrap();
    assert!(!events1.is_empty(), "first call should produce events");

    // Verify agent run was created
    let agent_runs = ctx.runtime.lock_state(|s| s.agent_runs.len());
    assert_eq!(agent_runs, 1, "should have exactly one agent run");

    // Second call: should be a no-op due to idempotency guard
    let events2 = ctx.runtime.handle_event(event).await.unwrap();
    assert!(
        events2.is_empty(),
        "second call should produce no events (idempotent)"
    );

    // Still exactly one agent run
    let agent_runs = ctx.runtime.lock_state(|s| s.agent_runs.len());
    assert_eq!(agent_runs, 1, "should still have exactly one agent run");
}
