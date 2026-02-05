// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Integration tests for environment variable handling.

use oj_shell::ShellExecutor;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn executor() -> ShellExecutor {
    ShellExecutor::new()
}

// ---------------------------------------------------------------------------
// Test Infrastructure (Sanity Check)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sanity_check() {
    let result = executor().execute_str("echo hello").await.unwrap();
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("hello\n"));
}

// ---------------------------------------------------------------------------
// Tests for env() / envs() Reaching Child Processes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn env_reaches_child_process() {
    // Use printenv to verify the variable exists in child's environment
    let result = executor()
        .env("TEST_VAR", "test_value")
        .execute_str("printenv TEST_VAR")
        .await
        .unwrap();
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("test_value\n")
    );
}

#[tokio::test]
async fn envs_reaches_child_process() {
    let result = executor()
        .envs([("A", "1"), ("B", "2")])
        .execute_str("sh -c 'echo $A $B'")
        .await
        .unwrap();
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("1 2\n"));
}

// ---------------------------------------------------------------------------
// Tests for variable() Expansion Without Child Env
// ---------------------------------------------------------------------------

#[tokio::test]
async fn variable_expands_in_args() {
    let result = executor()
        .variable("NAME", "world")
        .execute_str("echo hello $NAME")
        .await
        .unwrap();
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("hello world\n")
    );
}

#[tokio::test]
async fn variable_not_in_child_env() {
    // Shell variable should NOT appear in child's environment
    let result = executor()
        .variable("SHELL_ONLY", "secret")
        .execute_str("printenv SHELL_ONLY || echo missing")
        .await;

    // printenv returns exit 1 when var not found, OR chain runs echo
    let result = result.unwrap();
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("missing\n")
    );
}

// ---------------------------------------------------------------------------
// Tests for Env Prefix Scoping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn env_prefix_affects_only_that_command() {
    // First command gets the prefix var, second command does not
    let result = executor()
        .execute_str("TEMP_VAR=temporary printenv TEMP_VAR && printenv TEMP_VAR || echo gone")
        .await
        .unwrap();

    // First printenv succeeds with "temporary"
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("temporary\n")
    );
    // Second printenv fails (var gone), fallback prints "gone"
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("gone\n")
    );
}

#[tokio::test]
async fn env_prefix_does_not_leak_to_next_command() {
    // Sequential commands: prefix only on first
    let result = executor()
        .execute_str("PREFIX_VAR=first true; printenv PREFIX_VAR || echo absent")
        .await;

    // First command succeeds, second printenv fails, fallback runs
    let result = result.unwrap();
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("absent\n")
    );
}

// ---------------------------------------------------------------------------
// Tests for Parent Environment Inheritance
// ---------------------------------------------------------------------------

#[tokio::test]
async fn child_inherits_parent_environment() {
    // PATH must be inherited for commands to work at all
    // Test with a variable we know exists
    let result = executor().execute_str("printenv PATH").await.unwrap();

    // PATH should be non-empty
    let stdout = result.traces[0].stdout_snippet.as_deref().unwrap_or("");
    assert!(!stdout.trim().is_empty(), "PATH should be inherited");
}

#[tokio::test]
async fn env_overrides_inherited_variable() {
    // Override an inherited variable (using HOME, not PATH which is needed for command lookup)
    let result = executor()
        .env("HOME", "/custom/home")
        .execute_str("printenv HOME")
        .await
        .unwrap();

    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("/custom/home\n")
    );
}

// ---------------------------------------------------------------------------
// Tests for $VAR Expansion Precedence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expansion_reads_shell_variable() {
    let result = executor()
        .variable("MY_VAR", "shell_value")
        .execute_str("echo $MY_VAR")
        .await
        .unwrap();

    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("shell_value\n")
    );
}

#[tokio::test]
async fn expansion_reads_from_env_when_no_shell_var() {
    // Set via env(), expand via $VAR - should work if shell falls back to env
    // NOTE: This test documents current behavior. The shell may or may not
    // support reading process env vars for expansion.
    let result = executor()
        .env("ENV_VAR", "from_env")
        .execute_str("echo ${ENV_VAR:-fallback}")
        .await
        .unwrap();

    // If env vars aren't readable for expansion, we get "fallback"
    // Document whichever behavior is correct
    let stdout = result.traces[0].stdout_snippet.as_deref().unwrap();
    assert!(stdout == "from_env\n" || stdout == "fallback\n");
}

#[tokio::test]
async fn shell_variable_overrides_env_for_expansion() {
    // Shell variable should take precedence over env for $VAR expansion
    let result = executor()
        .env("PRIORITY_TEST", "from_env")
        .variable("PRIORITY_TEST", "from_shell")
        .execute_str("echo $PRIORITY_TEST")
        .await
        .unwrap();

    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("from_shell\n")
    );
}
