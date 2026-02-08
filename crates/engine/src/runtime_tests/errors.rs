// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Error handling tests

use super::*;
use oj_core::JobId;

#[tokio::test]
async fn command_not_found_returns_error() {
    let ctx = setup().await;

    let result = ctx
        .runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "nonexistent",
            HashMap::new(),
            &ctx.project_root,
        ))
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nonexistent"));
}

#[tokio::test]
async fn shell_completed_for_unknown_job_errors() {
    let ctx = setup().await;

    let result = ctx
        .runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new("nonexistent"),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await;

    assert!(result.is_err());
}

/// Runbook where a step references a nonexistent job definition
const RUNBOOK_MISSING_JOB_DEF: &str = r#"
[command.build]
args = "<name>"
run = { job = "nonexistent" }
"#;

#[tokio::test]
async fn command_referencing_nonexistent_job_errors() {
    let ctx = setup_with_runbook(RUNBOOK_MISSING_JOB_DEF).await;

    let result = ctx
        .runtime
        .handle_event(command_event(
            "pipe-1",
            "nonexistent",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("nonexistent"));
}

/// Runbook with workspace mode to test workspace setup failures
const RUNBOOK_WITH_WORKSPACE: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]
workspace = "folder"

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn workspace_job_creates_directory() {
    // Workspace creation is deferred: CommandRun emits JobCreated,
    // which triggers handle_job_created → CreateWorkspace.
    // WorkspaceReady arrives asynchronously via event_tx.
    let mut ctx = setup_with_runbook(RUNBOOK_WITH_WORKSPACE).await;

    let events = ctx
        .runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test-ws".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    // Job should exist with step_status=Pending (waiting for workspace)
    let job = ctx
        .runtime
        .get_job("pipe-1")
        .expect("job should exist in state");
    assert_eq!(job.step, "init");
    assert_eq!(
        job.step_status,
        oj_core::StepStatus::Pending,
        "step should be pending until workspace is ready"
    );

    // Process the JobCreated event to trigger workspace creation
    for event in events {
        ctx.runtime.handle_event(event).await.unwrap();
    }

    // Wait for WorkspaceReady from the background task
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), ctx.event_rx.recv())
        .await
        .expect("timed out waiting for workspace event")
        .expect("channel closed");
    assert!(
        matches!(event, Event::WorkspaceReady { .. }),
        "expected WorkspaceReady, got: {:?}",
        event
    );

    // Workspace directory should have been created
    let workspaces_dir = ctx.project_root.join("workspaces");
    assert!(workspaces_dir.exists(), "workspaces dir should be created");
}

/// Runbook where a step references a nonexistent agent
const RUNBOOK_MISSING_AGENT: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = { agent = "nonexistent" }
"#;

#[tokio::test]
async fn step_referencing_nonexistent_agent_errors() {
    let ctx = setup_with_runbook(RUNBOOK_MISSING_AGENT).await;

    let result = ctx
        .runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("nonexistent"));
}
