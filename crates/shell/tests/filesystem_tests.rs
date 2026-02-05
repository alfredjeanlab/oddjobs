// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Integration tests for shell filesystem side-effects.
//!
//! Tests verify that shell scripts correctly create, modify, and read files
//! through redirections. All tests use tempfile for isolation.

use oj_shell::{ExecError, ShellExecutor};
use std::fs;
use tempfile::TempDir;

/// Create an executor with cwd set to the temp directory.
fn executor_in(dir: &TempDir) -> ShellExecutor {
    ShellExecutor::new().cwd(dir.path())
}

// ---------------------------------------------------------------------------
// Phase 2: File creation with redirections
// ---------------------------------------------------------------------------

#[tokio::test]
async fn redirect_creates_file_in_cwd() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    let result = exec.execute_str("echo hello > out.txt").await.unwrap();
    assert_eq!(result.exit_code, 0);

    let content = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(content.trim(), "hello");
}

#[tokio::test]
async fn redirect_overwrites_existing_file() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    // Create initial file
    exec.execute_str("echo first > out.txt").await.unwrap();
    // Overwrite with new content
    exec.execute_str("echo second > out.txt").await.unwrap();

    let content = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(content.trim(), "second");
}

#[tokio::test]
async fn redirect_creates_nested_path() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("subdir")).unwrap();
    let exec = executor_in(&tmp);

    exec.execute_str("echo nested > subdir/file.txt")
        .await
        .unwrap();

    let content = fs::read_to_string(tmp.path().join("subdir/file.txt")).unwrap();
    assert_eq!(content.trim(), "nested");
}

#[tokio::test]
async fn redirect_to_nonexistent_dir_fails() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    let err = exec
        .execute_str("echo fail > nonexistent/out.txt")
        .await
        .unwrap_err();
    assert!(matches!(err, ExecError::RedirectFailed { .. }));
}

// ---------------------------------------------------------------------------
// Phase 3: Append mode redirections
// ---------------------------------------------------------------------------

#[tokio::test]
async fn append_creates_file_if_missing() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    exec.execute_str("echo first >> out.txt").await.unwrap();

    let content = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    assert_eq!(content.trim(), "first");
}

#[tokio::test]
async fn append_preserves_existing_content() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    exec.execute_str("echo line1 >> out.txt").await.unwrap();
    exec.execute_str("echo line2 >> out.txt").await.unwrap();
    exec.execute_str("echo line3 >> out.txt").await.unwrap();

    let content = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    let lines: Vec<_> = content.lines().collect();
    assert_eq!(lines, vec!["line1", "line2", "line3"]);
}

#[tokio::test]
async fn append_and_overwrite_in_sequence() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    // Overwrite -> append -> overwrite -> append
    exec.execute_str("echo a > out.txt").await.unwrap();
    exec.execute_str("echo b >> out.txt").await.unwrap();
    exec.execute_str("echo c > out.txt").await.unwrap();
    exec.execute_str("echo d >> out.txt").await.unwrap();

    let content = fs::read_to_string(tmp.path().join("out.txt")).unwrap();
    let lines: Vec<_> = content.lines().collect();
    assert_eq!(lines, vec!["c", "d"]);
}

// ---------------------------------------------------------------------------
// Phase 4: Reading from files
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_from_created_file() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    // Create file then read it back
    exec.execute_str("echo 'test content' > input.txt")
        .await
        .unwrap();
    let result = exec.execute_str("cat < input.txt").await.unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("test content\n")
    );
}

#[tokio::test]
async fn read_from_nonexistent_file_fails() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    let err = exec.execute_str("cat < missing.txt").await.unwrap_err();
    assert!(matches!(err, ExecError::RedirectFailed { .. }));
}

#[tokio::test]
async fn chained_read_write() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    // Create input, transform, write output
    fs::write(tmp.path().join("input.txt"), "hello world\n").unwrap();
    exec.execute_str("cat < input.txt > output.txt")
        .await
        .unwrap();

    let content = fs::read_to_string(tmp.path().join("output.txt")).unwrap();
    assert_eq!(content, "hello world\n");
}

#[tokio::test]
async fn multiline_file_handling() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    let input = "line one\nline two\nline three\n";
    fs::write(tmp.path().join("multi.txt"), input).unwrap();

    let result = exec.execute_str("cat < multi.txt").await.unwrap();
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some(input));
}

// ---------------------------------------------------------------------------
// Phase 5: Cleanup and isolation tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn temp_dir_isolation() {
    // Two tests with same filename don't interfere
    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();

    executor_in(&tmp1)
        .execute_str("echo one > shared.txt")
        .await
        .unwrap();
    executor_in(&tmp2)
        .execute_str("echo two > shared.txt")
        .await
        .unwrap();

    let content1 = fs::read_to_string(tmp1.path().join("shared.txt")).unwrap();
    let content2 = fs::read_to_string(tmp2.path().join("shared.txt")).unwrap();

    assert_eq!(content1.trim(), "one");
    assert_eq!(content2.trim(), "two");
}

#[tokio::test]
async fn files_cleaned_up_when_tempdir_dropped() {
    let path = {
        let tmp = TempDir::new().unwrap();
        let exec = executor_in(&tmp);
        exec.execute_str("echo test > cleanup.txt").await.unwrap();

        // Verify file exists
        assert!(tmp.path().join("cleanup.txt").exists());
        tmp.path().to_path_buf()
    };
    // TempDir dropped, directory should be cleaned up
    assert!(!path.exists());
}

#[tokio::test]
async fn partial_execution_leaves_created_files() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    // First command succeeds, second fails
    let _ = exec.execute_str("echo created > success.txt; false").await;

    // File from first command should exist
    assert!(tmp.path().join("success.txt").exists());
    let content = fs::read_to_string(tmp.path().join("success.txt")).unwrap();
    assert_eq!(content.trim(), "created");
}

#[tokio::test]
async fn stderr_redirect_creates_file() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    // Redirect stderr to file (using a command that writes to stderr)
    exec.execute_str("sh -c 'echo error >&2' 2> err.txt")
        .await
        .unwrap();

    let content = fs::read_to_string(tmp.path().join("err.txt")).unwrap();
    assert_eq!(content.trim(), "error");
}

#[tokio::test]
async fn both_redirect_creates_file() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp);

    // &> redirects both stdout and stderr
    exec.execute_str("sh -c 'echo out; echo err >&2' &> both.txt")
        .await
        .unwrap();

    let content = fs::read_to_string(tmp.path().join("both.txt")).unwrap();
    assert!(content.contains("out"));
    assert!(content.contains("err"));
}

// ---------------------------------------------------------------------------
// Phase 6: Variable expansion in paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn variable_in_redirect_path() {
    let tmp = TempDir::new().unwrap();
    let exec = executor_in(&tmp).variable("OUTFILE", "dynamic.txt");

    exec.execute_str("echo content > $OUTFILE").await.unwrap();

    let content = fs::read_to_string(tmp.path().join("dynamic.txt")).unwrap();
    assert_eq!(content.trim(), "content");
}

#[tokio::test]
async fn shell_var_in_redirect_path() {
    let tmp = TempDir::new().unwrap();
    // Note: use .variable() for shell expansion, not .env()
    // .env() passes vars to child processes; .variable() is for $VAR expansion
    let exec = executor_in(&tmp).variable("MY_OUTPUT", "var-file.txt");

    exec.execute_str("echo data > $MY_OUTPUT").await.unwrap();

    let content = fs::read_to_string(tmp.path().join("var-file.txt")).unwrap();
    assert_eq!(content.trim(), "data");
}
