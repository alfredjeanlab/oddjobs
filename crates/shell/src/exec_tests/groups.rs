// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for subshell and brace group execution.

use super::{executor, run_async};
use crate::exec::{ExecError, ShellExecutor};
use crate::Parser;

// ---------------------------------------------------------------------------
// Subshell
// ---------------------------------------------------------------------------

#[tokio::test]
async fn subshell_does_not_leak_variables() {
    // In a subshell, variable changes don't escape.
    // We test indirectly: run a subshell that succeeds, then check it didn't
    // error.  (Full variable-leak test requires builtins we don't intercept.)
    let result = executor().execute_str("(echo inside)").await.unwrap();
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn subshell_redirect_to_file() {
    let dir = std::env::temp_dir().join("oj_shell_test_subshell_redirect");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.txt");

    let script = format!("(echo hello) > {}", file.display());
    let result = executor().execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content.trim(), "hello");

    // stdout was redirected to file, so traces should not contain it
    for trace in &result.traces {
        assert!(trace.stdout_snippet.is_none());
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn subshell_inherits_parent_variables() {
    // Subshell should have access to variables from the parent context
    let result = ShellExecutor::new()
        .variable("VAR", "outer")
        .execute_str("(echo $VAR)")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    // The subshell should see the parent's variable
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("outer\n")
    );
}

#[tokio::test]
async fn subshell_variable_not_visible_outside() {
    // Variable set via := modifier inside subshell should NOT be visible after
    // (subshells correctly isolate their environment)
    // Use ${VAR:-} to explicitly allow empty value (nounset behavior)
    let result = executor()
        .execute_str("(echo ${VAR:=hello}); echo ${VAR:-}")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 2);
    // First echo inside subshell
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("hello\n"));
    // Second echo outside subshell should NOT see the variable (empty)
    assert_eq!(result.traces[1].stdout_snippet.as_deref(), Some("\n"));
}

// ---------------------------------------------------------------------------
// Brace groups
// ---------------------------------------------------------------------------

#[tokio::test]
async fn brace_group_runs() {
    let result = executor().execute_str("{ echo a; echo b; }").await.unwrap();
    assert_eq!(result.exit_code, 0);
}

#[tokio::test]
async fn brace_group_redirect_to_file() {
    let dir = std::env::temp_dir().join("oj_shell_test_brace_redirect");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.txt");

    let script = format!("{{ echo hello; }} > {}", file.display());
    let result = executor().execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content.trim(), "hello");

    // stdout was redirected to file, so traces should not contain it
    for trace in &result.traces {
        assert!(trace.stdout_snippet.is_none());
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn brace_group_variable_visible_outside() {
    // Variable set via := modifier inside brace group should be visible after
    let result = executor()
        .execute_str("{ echo ${VAR:=hello}; }; echo $VAR")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 2);
    // First echo inside brace group
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("hello\n"));
    // Second echo outside brace group should see the variable set inside
    assert_eq!(result.traces[1].stdout_snippet.as_deref(), Some("hello\n"));
}

// ---------------------------------------------------------------------------
// Group failure
// ---------------------------------------------------------------------------

/// Tests that failing commands in subshells/brace groups return CommandFailed.
#[yare::parameterized(
    subshell = { "(false)" },
    brace_group = { "{ false; }" },
)]
fn group_failure_returns_exit_code(script: &str) {
    run_async(async {
        let ast = Parser::parse(script).unwrap();
        let err = executor().execute(&ast).await.unwrap_err();
        match err {
            ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 1),
            other => panic!("expected CommandFailed, got: {other:?}"),
        }
    });
}

// ---------------------------------------------------------------------------
// Nested groups
// ---------------------------------------------------------------------------

/// Tests for nested subshells and brace groups.
#[yare::parameterized(
    nested_subshells = { "((echo nested))", "nested\n" },
    nested_brace_groups = { "{ { echo nested; }; }", "nested\n" },
    subshell_containing_brace = { "({ echo mixed; })", "mixed\n" },
    brace_containing_subshell = { "{ (echo mixed); }", "mixed\n" },
    deeply_nested_subshells = { "(((echo deep)))", "deep\n" },
    deeply_nested_braces = { "{ { { echo deep; }; }; }", "deep\n" },
)]
fn nested_groups(script: &str, expected: &str) {
    run_async(async {
        let result = executor().execute_str(script).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(
            result.traces.last().unwrap().stdout_snippet.as_deref(),
            Some(expected)
        );
    });
}
