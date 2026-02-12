// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;

use tempfile::tempdir;

use crate::protocol::Response;

use super::super::test_ctx;
use super::{handle_run_command, RunCommandParams};

/// Helper: create a temp project with a runbook TOML and return the project root path.
fn project_with_runbook(toml_content: &str) -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let runbook_dir = dir.path().join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(runbook_dir.join("test.toml"), toml_content).unwrap();
    dir
}

#[tokio::test]
async fn shell_command_uses_command_name_as_job_name() {
    let project = project_with_runbook(
        r#"
[command.deploy]
run = "echo deploying"
"#,
    );

    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

    let result = handle_run_command(RunCommandParams {
        project_path: project.path(),
        invoke_dir: project.path(),
        project: "",
        command: "deploy",
        args: &[],
        named_args: &HashMap::new(),
        ctx: &ctx,
    })
    .await
    .unwrap();

    match result {
        Response::JobStarted { job_name, .. } => {
            assert_eq!(job_name, "deploy");
        }
        other => panic!("expected JobStarted, got {:?}", other),
    }
}

#[tokio::test]
async fn job_command_uses_job_name() {
    let project = project_with_runbook(
        r#"
[command.build]
run = { job = "build-all" }

[job.build-all]
input  = []

[[job.build-all.step]]
name = "compile"
run = "make"
"#,
    );

    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

    let result = handle_run_command(RunCommandParams {
        project_path: project.path(),
        invoke_dir: project.path(),
        project: "",
        command: "build",
        args: &[],
        named_args: &HashMap::new(),
        ctx: &ctx,
    })
    .await
    .unwrap();

    match result {
        Response::JobStarted { job_name, .. } => {
            assert_eq!(job_name, "build-all");
        }
        other => panic!("expected JobStarted, got {:?}", other),
    }
}

#[tokio::test]
async fn unknown_command_suggests_similar_name() {
    let project = project_with_runbook(
        r#"
[command.deploy]
run = "echo deploying"
"#,
    );

    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

    let result = handle_run_command(RunCommandParams {
        project_path: project.path(),
        invoke_dir: project.path(),
        project: "",
        command: "deploj",
        args: &[],
        named_args: &HashMap::new(),
        ctx: &ctx,
    })
    .await
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("did you mean: deploy?")),
        "expected suggestion for 'deploy', got {:?}",
        result
    );
}

#[tokio::test]
async fn unknown_command_returns_error_without_hint_when_no_match() {
    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

    let result = handle_run_command(RunCommandParams {
        project_path: std::path::Path::new("/nonexistent"),
        invoke_dir: std::path::Path::new("/nonexistent"),
        project: "",
        command: "xyz",
        args: &[],
        named_args: &HashMap::new(),
        ctx: &ctx,
    })
    .await
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("unknown command: xyz")),
        "expected unknown command error, got {:?}",
        result
    );
}
