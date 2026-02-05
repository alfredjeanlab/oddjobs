// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for jobs, pipefail, and multi-stage jobs.

use super::{executor, run_async};
use crate::exec::{ExecError, ShellExecutor};
use crate::Parser;

// ---------------------------------------------------------------------------
// Basic jobs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn job_echo_cat() {
    let result = executor().execute_str("echo hello | cat").await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 2);
    // Last command (cat) should have captured the output.
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("hello\n")
    );
}

#[tokio::test]
async fn job_exit_code_from_last() {
    // `false | true` -> job exit = 0 (last command)
    let result = executor().execute_str("false | true").await.unwrap();
    assert_eq!(result.exit_code, 0);

    // `true | false` -> job exit = 1 (last command)
    let ast = Parser::parse("true | false").unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    match err {
        ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 1),
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Pipefail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn job_pipefail_returns_rightmost_failure() {
    // `false | true` with pipefail -> 1 (rightmost failure is from `false`)
    let executor = ShellExecutor::new().pipefail(true);
    let err = executor.execute_str("false | true").await.unwrap_err();
    match err {
        ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 1),
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn job_pipefail_all_success_returns_zero() {
    // `true | true` with pipefail -> 0
    let executor = ShellExecutor::new().pipefail(true);
    let result = executor.execute_str("true | true").await.unwrap();
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn job_pipefail_multiple_failures_returns_rightmost() {
    // `exit 2 | exit 3 | true` with pipefail -> 3 (rightmost failure)
    let executor = ShellExecutor::new().pipefail(true);
    let err = executor
        .execute_str("sh -c 'exit 2' | sh -c 'exit 3' | true")
        .await
        .unwrap_err();
    match err {
        ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 3),
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Multi-stage jobs (3+ commands)
// ---------------------------------------------------------------------------

#[yare::parameterized(
    three_stages = { "echo hello | cat | cat", 3, "hello\n" },
    four_stages = { "echo a | cat | cat | cat", 4, "a\n" },
    transforms = { "echo abc | tr a X | tr b Y | tr c Z", 4, "XYZ\n" },
    head_tail = { "printf 'line1\\nline2\\nline3' | head -n 2 | tail -n 1", 3, "line2\n" },
)]
fn job_stages(script: &str, trace_count: usize, expected: &str) {
    run_async(async {
        let result = executor().execute_str(script).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.traces.len(), trace_count);
        assert_eq!(
            result.traces.last().unwrap().stdout_snippet.as_deref(),
            Some(expected)
        );
    });
}

#[tokio::test]
async fn job_multi_stage_exit_code_from_last() {
    // Multi-stage job exit code comes from last command
    // First commands fail but last succeeds -> overall success
    let result = executor()
        .execute_str("false | false | false | true")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);

    // Last command fails -> overall failure
    let ast = Parser::parse("true | true | true | false").unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    match err {
        ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 1),
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

#[yare::parameterized(
    rightmost_failure = { "sh -c 'exit 2' | sh -c 'exit 3' | sh -c 'exit 4' | true", 4 },
    middle_failure = { "true | false | true | true", 1 },
)]
fn job_multi_stage_pipefail(script: &str, expected_exit: i32) {
    run_async(async {
        let executor = ShellExecutor::new().pipefail(true);
        let err = executor.execute_str(script).await.unwrap_err();
        match err {
            ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, expected_exit),
            other => panic!("expected CommandFailed, got: {other:?}"),
        }
    });
}

#[tokio::test]
async fn job_multi_stage_pipefail_all_success() {
    // All stages succeed with pipefail
    let executor = ShellExecutor::new().pipefail(true);
    let result = executor
        .execute_str("true | true | true | true")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
}
