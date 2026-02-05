// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for error paths, traces, and unsupported features.

use super::executor;
use crate::exec::ExecError;
use crate::Parser;

// ---------------------------------------------------------------------------
// Error spans
// ---------------------------------------------------------------------------

#[tokio::test]
async fn error_span_matches_failing_command() {
    let script = "false";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    let span = err.span();
    // Span should cover "false" (bytes 0..5)
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 5);
}

// ---------------------------------------------------------------------------
// Unsupported features
// ---------------------------------------------------------------------------

#[tokio::test]
async fn background_returns_unsupported() {
    let script = "echo hello &";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    match err {
        ExecError::Unsupported { feature, .. } => {
            assert!(feature.contains("background"), "feature = {feature}");
        }
        other => panic!("expected Unsupported, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Spawn failures
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spawn_failed_command_not_found() {
    let ast = Parser::parse("nonexistent_command_xyz_12345").unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    match err {
        ExecError::SpawnFailed {
            command, source, ..
        } => {
            assert_eq!(command, "nonexistent_command_xyz_12345");
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected SpawnFailed, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Redirect failures
// ---------------------------------------------------------------------------

#[tokio::test]
async fn redirect_failed_input_file_missing() {
    let ast = Parser::parse("cat < /nonexistent/path/to/file_xyz_12345.txt").unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    match err {
        ExecError::RedirectFailed {
            message, source, ..
        } => {
            assert!(
                message.contains("/nonexistent/path/to/file_xyz_12345.txt"),
                "message should contain the path: {message}"
            );
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected RedirectFailed, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Parse errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parse_error_propagates_through_execute_str() {
    // Malformed input: job with missing command after pipe
    let err = executor().execute_str("echo |").await.unwrap_err();
    match err {
        ExecError::Parse(parse_err) => {
            // Should be UnexpectedEof since we're missing a command after |
            assert!(
                matches!(parse_err, crate::ParseError::UnexpectedEof { .. }),
                "expected UnexpectedEof, got: {parse_err:?}"
            );
        }
        other => panic!("expected Parse error, got: {other:?}"),
    }
}
