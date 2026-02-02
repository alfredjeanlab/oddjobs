// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Error handling tests

use super::*;
use oj_core::PipelineId;

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
async fn shell_completed_for_unknown_pipeline_errors() {
    let ctx = setup().await;

    let result = ctx
        .runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new("nonexistent"),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await;

    assert!(result.is_err());
}

/// Runbook where a step references a nonexistent pipeline definition
const RUNBOOK_MISSING_PIPELINE_DEF: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "nonexistent" }
"#;

#[tokio::test]
async fn command_referencing_nonexistent_pipeline_errors() {
    let ctx = setup_with_runbook(RUNBOOK_MISSING_PIPELINE_DEF).await;

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
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]
workspace = "ephemeral"

[[pipeline.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn workspace_pipeline_creates_directory() {
    // With the refactored workspace approach, workspace creation uses mkdir
    // which is very unlikely to fail, so we test the happy path instead.
    let ctx = setup_with_runbook(RUNBOOK_WITH_WORKSPACE).await;

    // Set OJ_STATE_DIR to point into the test temp dir so workspace
    // is created in the test directory
    let state_dir = ctx.project_root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::env::set_var("OJ_STATE_DIR", &state_dir);

    let result = ctx
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
        .await;

    std::env::remove_var("OJ_STATE_DIR");

    assert!(result.is_ok(), "expected Ok, got: {:?}", result);

    // Pipeline should exist and be running
    let pipeline = ctx
        .runtime
        .get_pipeline("pipe-1")
        .expect("pipeline should exist in state");
    assert_eq!(pipeline.step, "init");

    // Workspace directory should have been created
    let workspaces_dir = state_dir.join("workspaces");
    assert!(workspaces_dir.exists(), "workspaces dir should be created");
}

/// Runbook where a step references a nonexistent agent
const RUNBOOK_MISSING_AGENT: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
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
