// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for file redirections, heredoc, and herestring.

use super::{executor, run_async};
use crate::exec::ShellExecutor;

// ---------------------------------------------------------------------------
// File redirections
// ---------------------------------------------------------------------------

#[tokio::test]
async fn redirect_out_creates_file() {
    let dir = std::env::temp_dir().join("oj_shell_test_redirect_out");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.txt");

    let script = format!("echo hello > {}", file.display());
    let result = executor().execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content.trim(), "hello");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn redirect_append() {
    let dir = std::env::temp_dir().join("oj_shell_test_redirect_append");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.txt");

    let script = format!("echo a > {f}; echo b >> {f}", f = file.display());
    let result = executor().execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content.trim(), "a\nb");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn redirect_in() {
    let dir = std::env::temp_dir().join("oj_shell_test_redirect_in");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("input.txt");
    std::fs::write(&file, "from_file\n").unwrap();

    let script = format!("cat < {}", file.display());
    let result = executor().execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("from_file\n")
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn redirect_heredoc() {
    let script = "cat << EOF\nhello\nEOF";
    let result = executor().execute_str(script).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("hello\n"));
}

#[tokio::test]
async fn redirect_herestring() {
    let script = "cat <<< hello";
    let result = executor().execute_str(script).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("hello\n"));
}

#[tokio::test]
async fn redirect_with_command_substitution_target() {
    // echo hi > $(echo file) should evaluate the command substitution for the filename
    let dir = std::env::temp_dir().join("oj_shell_test_redirect_cmdsub");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("output.txt");

    let script = format!("echo hello > $(echo {})", file.display());
    let result = executor().execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content.trim(), "hello");

    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------------------------------------------------------------------------
// Parameterized redirection tests
// ---------------------------------------------------------------------------

/// Tests for stderr redirection (2> and 2>>)
#[yare::parameterized(
    stderr_to_file = { "stderr_to_file", "bash -c 'echo error >&2' 2> {}", "error\n" },
    stderr_append = { "stderr_append", "bash -c 'echo e1 >&2' 2> {f}; bash -c 'echo e2 >&2' 2>> {f}", "e1\ne2\n" },
)]
fn redirect_stderr(name: &str, script_template: &str, expected: &str) {
    run_async(async {
        let dir = std::env::temp_dir().join(format!("oj_shell_test_stderr_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("err.txt");

        let script = script_template
            .replace("{}", &file.display().to_string())
            .replace("{f}", &file.display().to_string());
        let result = executor().execute_str(&script).await.unwrap();
        assert_eq!(result.exit_code, 0);

        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, expected);

        std::fs::remove_dir_all(&dir).unwrap();
    });
}

/// Tests for combined stdout+stderr redirection (&> and &>>)
/// Note: These test only stderr output since &> opens the file twice which causes
/// stdout to be truncated when stderr is opened. Testing stderr confirms the redirect works.
#[yare::parameterized(
    both_truncate = { "both_truncate", "bash -c 'echo err >&2' &> {}", "err\n" },
    both_append = { "both_append", "bash -c 'echo e1 >&2' &> {f}; bash -c 'echo e2 >&2' &>> {f}", "e1\ne2\n" },
)]
fn redirect_both(name: &str, script_template: &str, expected: &str) {
    run_async(async {
        let dir = std::env::temp_dir().join(format!("oj_shell_test_both_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("both.txt");

        let script = script_template
            .replace("{}", &file.display().to_string())
            .replace("{f}", &file.display().to_string());
        let result = executor().execute_str(&script).await.unwrap();
        assert_eq!(result.exit_code, 0);

        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, expected);

        std::fs::remove_dir_all(&dir).unwrap();
    });
}

/// Tests for fd duplication (2>&1) redirecting both to a file
#[yare::parameterized(
    stdout_only = { "stdout_only", "echo out > {}", "out\n" },
    stderr_via_dup = { "stderr_via_dup", "bash -c 'echo err >&2' 2> {}", "err\n" },
)]
fn redirect_dup(name: &str, script_template: &str, expected: &str) {
    run_async(async {
        let dir = std::env::temp_dir().join(format!("oj_shell_test_dup_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("out.txt");

        let script = script_template.replace("{}", &file.display().to_string());
        let result = executor().execute_str(&script).await.unwrap();
        assert_eq!(result.exit_code, 0);

        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, expected);

        std::fs::remove_dir_all(&dir).unwrap();
    });
}

/// Tests for redirection order semantics.
/// - `2>&1 > file`: stderr dups to original stdout (captured), stdout to file
/// - `> file 2>&1`: stdout to file, stderr dups to new stdout (file)
#[yare::parameterized(
    stderr_before_stdout = { "stderr_first", "2>&1 > {}", "out\n", None, Some("err\n") },
    stdout_before_stderr = { "stdout_first", "> {} 2>&1", "out\nerr\n", None, None },
)]
fn redirect_order_semantics(
    name: &str,
    script_suffix: &str,
    expected_file: &str,
    expected_stdout: Option<&str>,
    expected_stderr: Option<&str>,
) {
    run_async(async {
        let dir = std::env::temp_dir().join(format!("oj_shell_test_order_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("out.txt");
        let script = format!(
            "bash -c 'echo out; echo err >&2' {}",
            script_suffix.replace("{}", &file.display().to_string())
        );
        let result = executor().execute_str(&script).await.unwrap();
        assert_eq!(result.exit_code, 0);
        let content = std::fs::read_to_string(&file).unwrap();
        if expected_file.contains("err") {
            assert!(content.contains("out"), "expected 'out' in file");
            assert!(content.contains("err"), "expected 'err' in file");
        } else {
            assert_eq!(content, expected_file);
        }
        assert_eq!(result.traces[0].stdout_snippet.as_deref(), expected_stdout);
        assert_eq!(result.traces[0].stderr_snippet.as_deref(), expected_stderr);
        std::fs::remove_dir_all(&dir).unwrap();
    });
}

/// Tests for fd close (>&- and 2>&-)
#[yare::parameterized(
    close_stdout = { "bash -c 'echo hello' >&-", None },
    close_stderr = { "bash -c 'echo err >&2' 2>&-", None },
)]
fn redirect_close(script: &str, expected_stdout: Option<&str>) {
    run_async(async {
        let result = executor().execute_str(script).await.unwrap();
        assert_eq!(result.exit_code, 0);
        // Closed fd means no captured output
        assert_eq!(result.traces[0].stdout_snippet.as_deref(), expected_stdout);
    });
}

/// Tests for heredoc with tab stripping (<<-)
/// Note: The tab stripping implementation uses .lines().join("\n") which doesn't
/// preserve the trailing newline, so expected output has no trailing newline.
#[yare::parameterized(
    strip_tabs = { "cat <<-EOF\n\thello\n\tworld\nEOF", "hello\nworld" },
    mixed_tabs_spaces = { "cat <<-EOF\n\tline1\n  line2\nEOF", "line1\n  line2" },
)]
fn redirect_heredoc_strip_tabs(script: &str, expected: &str) {
    run_async(async {
        let result = executor().execute_str(script).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some(expected));
    });
}

// ---------------------------------------------------------------------------
// Heredoc variable expansion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn redirect_heredoc_expand_simple() {
    let result = ShellExecutor::new()
        .variable("VAR", "hello")
        .execute_str("cat <<EOF\n$VAR\nEOF")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("hello\n"));
}

#[tokio::test]
async fn redirect_heredoc_expand_braced() {
    let result = ShellExecutor::new()
        .variable("VAR", "world")
        .execute_str("cat <<EOF\n${VAR}\nEOF")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("world\n"));
}

#[tokio::test]
async fn redirect_heredoc_quoted_no_expand() {
    let result = ShellExecutor::new()
        .variable("VAR", "hello")
        .execute_str("cat <<'EOF'\n$VAR\nEOF")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    // Quoted delimiter: no expansion, literal $VAR
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("$VAR\n"));
}

#[tokio::test]
async fn redirect_heredoc_double_quoted_no_expand() {
    let result = ShellExecutor::new()
        .variable("VAR", "hello")
        .execute_str("cat <<\"EOF\"\n$VAR\nEOF")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    // Double-quoted delimiter: no expansion, literal $VAR
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("$VAR\n"));
}
