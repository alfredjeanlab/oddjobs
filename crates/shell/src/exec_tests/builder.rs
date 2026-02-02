// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for ShellExecutor builder methods.

use super::executor;
use crate::exec::ShellExecutor;

// ---------------------------------------------------------------------------
// Working directory
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cwd_changes_working_directory() {
    let dir = std::env::temp_dir().join("oj_shell_test_cwd");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Canonicalize to resolve symlinks (e.g., /var -> /private/var on macOS)
    let canonical_dir = dir.canonicalize().unwrap();

    let result = ShellExecutor::new()
        .cwd(&dir)
        .execute_str("pwd")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let output = result.traces[0].stdout_snippet.as_deref().unwrap().trim();
    assert_eq!(output, canonical_dir.to_str().unwrap());

    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------------------------------------------------------------------------
// Environment variables
// ---------------------------------------------------------------------------

#[tokio::test]
async fn env_passes_variable_to_process() {
    let result = ShellExecutor::new()
        .env("TEST_VAR", "test_value")
        .execute_str("printenv TEST_VAR")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("test_value\n")
    );
}

#[tokio::test]
async fn envs_passes_multiple_variables_to_process() {
    let result = ShellExecutor::new()
        .envs([("VAR_A", "alpha"), ("VAR_B", "beta")])
        .execute_str("bash -c 'echo $VAR_A $VAR_B'")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("alpha beta\n")
    );
}

// ---------------------------------------------------------------------------
// Shell variables
// ---------------------------------------------------------------------------

#[tokio::test]
async fn variables_sets_multiple_shell_variables() {
    let result = ShellExecutor::new()
        .variables([("X", "one"), ("Y", "two"), ("Z", "three")])
        .execute_str("echo $X $Y $Z")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("one two three\n")
    );
}

// ---------------------------------------------------------------------------
// Snippet limit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snippet_limit_truncates_output() {
    // Generate output larger than the limit
    let result = ShellExecutor::new()
        .snippet_limit(10)
        .execute_str("echo 'hello world this is a long string'")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let snippet = result.traces[0].stdout_snippet.as_deref().unwrap();
    // The output should be truncated to approximately 10 bytes
    assert!(
        snippet.len() <= 15,
        "snippet too long: {} bytes",
        snippet.len()
    );
}

#[tokio::test]
async fn snippet_limit_respects_utf8_boundaries() {
    // Use multi-byte characters (Japanese hiragana, 3 bytes each)
    // "aiueo" = 5 characters = 15 bytes
    // With limit of 10, it should truncate to 3 characters (9 bytes), not split a char
    let result = ShellExecutor::new()
        .snippet_limit(10)
        .execute_str("printf 'あいうえお'")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let snippet = result.traces[0].stdout_snippet.as_deref().unwrap();

    // Should be truncated at a character boundary (9 bytes for 3 Japanese chars)
    // The limit is 10, but 4 chars would be 12 bytes, so it backs up to 9 bytes
    assert_eq!(
        snippet.len(),
        9,
        "expected 9 bytes (3 chars), got {}",
        snippet.len()
    );
    assert_eq!(snippet, "あいう", "expected first 3 characters");
}

// ---------------------------------------------------------------------------
// Traces
// ---------------------------------------------------------------------------

#[tokio::test]
async fn traces_contain_timing_and_exit_codes() {
    let result = executor().execute_str("true && echo hi").await.unwrap();
    assert_eq!(result.traces.len(), 2);
    for trace in &result.traces {
        assert!(trace.duration.as_nanos() > 0);
    }
    assert_eq!(result.traces[0].exit_code, 0);
    assert_eq!(result.traces[1].exit_code, 0);
}
