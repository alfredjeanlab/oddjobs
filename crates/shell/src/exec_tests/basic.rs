// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for simple commands, exit codes, AND/OR chains, and fail-fast behavior.

use super::{executor, run_async};
use crate::exec::ExecError;
use crate::Parser;

// ---------------------------------------------------------------------------
// Simple commands
// ---------------------------------------------------------------------------

#[tokio::test]
async fn simple_echo() {
    let result = executor().execute_str("echo hello").await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 1);
    assert_eq!(result.traces[0].command, "echo");
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("hello\n"));
}

#[tokio::test]
async fn single_quote_escape_idiom() {
    // The '\'' idiom: end quote, escaped quote, start quote
    let result = executor()
        .execute_str("echo 'hello:it'\\''s'")
        .await
        .unwrap();
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("hello:it's\n")
    );
}

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

#[yare::parameterized(
    true_cmd = { "true", true },
    false_cmd = { "false", false },
)]
fn exit_code(script: &str, should_succeed: bool) {
    run_async(async {
        let ast = Parser::parse(script).unwrap();
        let result = executor().execute(&ast).await;
        if should_succeed {
            assert_eq!(result.unwrap().exit_code, 0);
        } else {
            match result.unwrap_err() {
                ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 1),
                other => panic!("expected CommandFailed, got: {other:?}"),
            }
        }
    });
}

// ---------------------------------------------------------------------------
// AND chains
// ---------------------------------------------------------------------------

#[tokio::test]
async fn and_chain_success() {
    let result = executor().execute_str("true && echo yes").await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 2);
    assert_eq!(result.traces[1].stdout_snippet.as_deref(), Some("yes\n"));
}

#[tokio::test]
async fn and_chain_skips_on_failure() {
    // `false && echo no` -- the echo should be skipped, exit code 1
    let ast = Parser::parse("false && echo no").unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    match err {
        ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 1),
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// OR chains
// ---------------------------------------------------------------------------

#[tokio::test]
async fn or_chain_runs_fallback() {
    let result = executor().execute_str("false || echo yes").await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("yes\n")
    );
}

#[tokio::test]
async fn or_chain_skips_on_success() {
    let result = executor().execute_str("true || echo no").await.unwrap();
    assert_eq!(result.exit_code, 0);
    // Only `true` ran, echo was skipped.
    assert_eq!(result.traces.len(), 1);
}

// ---------------------------------------------------------------------------
// Fail-fast
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fail_fast_stops_execution() {
    // `false; echo never` -- echo should NOT run due to fail-fast.
    let ast = Parser::parse("false; echo never").unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    match &err {
        ExecError::CommandFailed {
            command, exit_code, ..
        } => {
            assert_eq!(*exit_code, 1);
            assert_eq!(command, "false");
        }
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}
