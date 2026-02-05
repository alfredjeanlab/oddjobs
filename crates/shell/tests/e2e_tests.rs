// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! End-to-end integration tests for the shell executor.
//!
//! These tests execute real commands to validate multi-line scripts
//! matching patterns found in production runbooks.

use oj_shell::{ExecError, ShellExecutor};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test Infrastructure (Phase 1)
// ---------------------------------------------------------------------------

/// Create an executor with a working directory in a temp folder.
fn executor_in(dir: &TempDir) -> ShellExecutor {
    ShellExecutor::new().cwd(dir.path())
}

/// Create a temp directory for test isolation.
fn test_dir() -> TempDir {
    tempfile::tempdir().expect("failed to create temp dir")
}

/// Get absolute path string for a file in the temp directory.
fn abs_path(dir: &TempDir, file: &str) -> String {
    dir.path().join(file).display().to_string()
}

// ---------------------------------------------------------------------------
// Phase 2: Sequential Command Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sequential_commands_all_succeed() {
    // Pattern: mkdir -p dir; echo content > file; cat file
    let dir = test_dir();
    let subdir = abs_path(&dir, "subdir");
    let file = abs_path(&dir, "subdir/file.txt");
    let script = format!(
        r#"
        mkdir -p {subdir}
        echo "hello" > {file}
        cat {file}
    "#
    );

    let result = executor_in(&dir).execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 3);
    assert_eq!(result.traces[2].stdout_snippet.as_deref(), Some("hello\n"));
}

#[tokio::test]
async fn sequential_commands_fail_fast() {
    // Pattern: command fails mid-script, subsequent commands don't run
    let dir = test_dir();
    let script = r#"
        echo "first"
        test -f nonexistent
        echo "never"
    "#;

    let err = executor_in(&dir).execute_str(script).await.unwrap_err();
    match err {
        ExecError::CommandFailed { command, .. } => {
            assert_eq!(command, "test");
        }
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn sequential_with_directory_operations() {
    // Pattern: mkdir/cd operations with nested directories
    let dir = test_dir();
    let nested = abs_path(&dir, "a/b/c");
    let file = abs_path(&dir, "a/b/c/file.txt");
    let script = format!(
        r#"
        mkdir -p {nested}
        test -d {nested}
        touch {file}
        test -f {file}
    "#
    );

    let result = executor_in(&dir).execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(dir.path().join("a/b/c/file.txt").exists());
}

#[tokio::test]
async fn sequential_file_creation_chain() {
    // Pattern: create, append, read
    let dir = test_dir();
    let file = abs_path(&dir, "chain.txt");
    let script = format!(
        r#"
        echo "line1" > {file}
        echo "line2" >> {file}
        echo "line3" >> {file}
        cat {file}
    "#
    );

    let result = executor_in(&dir).execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let output = result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap();
    assert!(output.contains("line1"));
    assert!(output.contains("line2"));
    assert!(output.contains("line3"));
}

// ---------------------------------------------------------------------------
// Phase 3: Job and Redirection Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn job_with_grep() {
    // Pattern: echo content | grep pattern
    let result = ShellExecutor::new()
        .execute_str("echo 'hello world' | grep world")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("world"));
}

#[tokio::test]
async fn job_multi_stage() {
    // Pattern: cat file | grep pattern | wc -l
    let dir = test_dir();
    let file = abs_path(&dir, "data.txt");
    std::fs::write(dir.path().join("data.txt"), "a\nb\nc\n").unwrap();

    let script = format!("cat {file} | grep -v b | wc -l");
    let result = executor_in(&dir).execute_str(&script).await.unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 3);
    // Should have 2 lines (a and c)
    assert!(result.traces[2]
        .stdout_snippet
        .as_deref()
        .unwrap()
        .trim()
        .contains('2'));
}

#[tokio::test]
async fn job_exit_from_last_command() {
    // Pattern: verify exit code semantics from last job stage

    // `echo hello | true` → exit 0
    let result = ShellExecutor::new()
        .execute_str("echo hello | true")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);

    // `echo hello | false` → exit 1
    let err = ShellExecutor::new()
        .execute_str("echo hello | false")
        .await
        .unwrap_err();
    match err {
        ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 1),
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn redirect_with_append_chain() {
    // Pattern from uat.toml: echo > file; echo >> file; cat file
    let dir = test_dir();
    let file = abs_path(&dir, "out.txt");
    let script = format!(
        r#"
        echo "line1" > {file}
        echo "line2" >> {file}
        echo "line3" >> {file}
        cat {file}
    "#
    );

    let result = executor_in(&dir).execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let output = result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap();
    assert!(output.contains("line1"));
    assert!(output.contains("line2"));
    assert!(output.contains("line3"));
}

#[tokio::test]
async fn redirect_input_file() {
    // Pattern: command < file
    let dir = test_dir();
    let file = abs_path(&dir, "input.txt");
    std::fs::write(dir.path().join("input.txt"), "file_content\n").unwrap();

    let script = format!("cat < {file}");
    let result = executor_in(&dir).execute_str(&script).await.unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("file_content\n")
    );
}

#[tokio::test]
async fn redirect_heredoc_multiline() {
    // Multi-line heredoc content
    let script = r#"cat << EOF
first line
second line
third line
EOF"#;

    let result = ShellExecutor::new().execute_str(script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let output = result.traces[0].stdout_snippet.as_deref().unwrap();
    assert!(output.contains("first line"));
    assert!(output.contains("second line"));
    assert!(output.contains("third line"));
}

#[tokio::test]
async fn redirect_herestring_with_pipe() {
    // Pattern: <<< string | cmd
    let result = ShellExecutor::new()
        .execute_str("cat <<< 'hello' | grep hello")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("hello"));
}

// ---------------------------------------------------------------------------
// Phase 4: Variable Expansion Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn variable_expansion_in_paths() {
    // Pattern: mkdir -p $DIR; echo > $DIR/file
    let dir = test_dir();
    let base = dir.path().display().to_string();
    let result = ShellExecutor::new()
        .cwd(dir.path())
        .variable("BASE", &base)
        .variable("SUBDIR", "mydir")
        .execute_str(
            r#"
            mkdir -p $BASE/$SUBDIR
            echo "content" > $BASE/$SUBDIR/file.txt
            cat $BASE/$SUBDIR/file.txt
        "#,
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(dir.path().join("mydir/file.txt").exists());
}

#[tokio::test]
async fn variable_expansion_with_defaults() {
    // Pattern: ${VAR:-default} when VAR unset
    let result = ShellExecutor::new()
        .execute_str("echo ${UNSET_VAR:-fallback}")
        .await
        .unwrap();

    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("fallback\n")
    );
}

#[tokio::test]
async fn variable_expansion_with_set_default() {
    // Pattern: ${VAR:=value} modifier - sets and uses default
    let result = ShellExecutor::new()
        .execute_str(r#"echo ${MYVAR:=default_val}"#)
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result.traces[0]
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("default_val"));
}

#[tokio::test]
async fn variable_expansion_with_alternate() {
    // Pattern: ${VAR:+alt} modifier - use alt if VAR is set
    let result = ShellExecutor::new()
        .variable("MYVAR", "exists")
        .execute_str("echo ${MYVAR:+alternate}")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result.traces[0]
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("alternate"));

    // When unset, should produce empty
    let result2 = ShellExecutor::new()
        .execute_str("echo ${UNSET:+alternate}")
        .await
        .unwrap();

    assert_eq!(result2.exit_code, 0);
    // Output should be just newline (empty expansion)
    assert_eq!(result2.traces[0].stdout_snippet.as_deref(), Some("\n"));
}

#[tokio::test]
async fn command_substitution_in_argument() {
    // Pattern: echo "Result: $(cmd)"
    let result = ShellExecutor::new()
        .execute_str(r#"echo "Value: $(echo 42)""#)
        .await
        .unwrap();

    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("Value: 42\n")
    );
}

#[tokio::test]
async fn command_substitution_nested() {
    // Pattern: $(echo $(echo val))
    let result = ShellExecutor::new()
        .execute_str("echo $(echo $(echo nested))")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result.traces[0]
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("nested"));
}

#[tokio::test]
async fn variables_across_commands() {
    // Variables set via .variable() persist across script commands
    let dir = test_dir();
    let base = dir.path().display().to_string();
    let result = ShellExecutor::new()
        .cwd(dir.path())
        .variable("BASE", &base)
        .variable("NAME", "test")
        .variable("EXT", "txt")
        .execute_str(
            r#"
            echo "header" > $BASE/$NAME.$EXT
            echo "body" >> $BASE/$NAME.$EXT
            cat $BASE/$NAME.$EXT
        "#,
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let content = std::fs::read_to_string(dir.path().join("test.txt")).unwrap();
    assert!(content.contains("header"));
    assert!(content.contains("body"));
}

#[tokio::test]
async fn env_passed_to_child_processes() {
    // .env() passes environment variables to child processes
    let result = ShellExecutor::new()
        .env("MY_TEST_ENV", "env_value")
        .execute_str("printenv MY_TEST_ENV")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("env_value\n")
    );
}

#[tokio::test]
async fn shell_variables_expand_in_arguments() {
    // .variable() is for shell expansion in arguments
    let result = ShellExecutor::new()
        .variable("MY_VAR", "var_value")
        .execute_str("echo $MY_VAR")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("var_value\n")
    );
}

// ---------------------------------------------------------------------------
// Phase 5: Subshell and Brace Group Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn subshell_isolation() {
    // Subshell changes don't affect parent
    let dir = test_dir();
    let result = executor_in(&dir)
        .execute_str(
            r#"
            (cd /tmp && echo "in subshell")
            pwd
        "#,
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    // pwd should still be in original dir, not /tmp
    let pwd_output = result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap();
    assert!(!pwd_output.contains("/tmp"));
}

#[tokio::test]
async fn subshell_exit_code() {
    // Exit code propagation from subshell using false command
    let err = ShellExecutor::new()
        .execute_str("(false)")
        .await
        .unwrap_err();

    match err {
        ExecError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, 1),
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn brace_group_sequential() {
    // Brace groups run in same context
    let result = ShellExecutor::new()
        .execute_str("{ echo a; echo b; echo c; }")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 3);
}

#[tokio::test]
async fn brace_group_with_redirect() {
    // Brace groups execute commands and capture output
    // Note: brace group level redirections are not yet implemented,
    // so we test redirect on individual commands within the group
    let dir = test_dir();
    let file = abs_path(&dir, "output.txt");
    let script = format!(
        r#"
        {{ echo "line1" > {file}; echo "line2" >> {file}; }}
        cat {file}
    "#
    );

    let result = executor_in(&dir).execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);

    let content = std::fs::read_to_string(dir.path().join("output.txt")).unwrap();
    assert!(content.contains("line1"));
    assert!(content.contains("line2"));
}

#[tokio::test]
async fn nested_subshells() {
    // Pattern: (cmd1; (cmd2))
    let result = ShellExecutor::new()
        .execute_str("(echo outer; (echo inner))")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
}

// ---------------------------------------------------------------------------
// Phase 6: Multi-Feature Integration Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn git_style_workflow() {
    // Pattern from build.toml: conditional operations with OR chains
    let dir = test_dir();
    let file = abs_path(&dir, "tracked.txt");
    let script = format!(
        r#"
        echo "file content" > {file}
        test -f {file} || false
        cat {file} | grep -q "content" || false
        echo "success"
    "#
    );

    let result = executor_in(&dir).execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("success"));
}

#[tokio::test]
async fn or_chain_with_exit() {
    // Pattern: command || fallback (conditional fallback)
    let script = "true || false; echo reached";
    let result = ShellExecutor::new().execute_str(script).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("reached"));
}

#[tokio::test]
async fn and_chain_with_commands() {
    // Pattern: cmd1 && cmd2 && cmd3 chains
    let dir = test_dir();
    let testdir = abs_path(&dir, "testdir");
    let file = abs_path(&dir, "testdir/file.txt");
    let script = format!(
        r#"
        mkdir -p {testdir}
        echo "created" > {file} && cat {file}
    "#
    );

    let result = executor_in(&dir).execute_str(&script).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("created"));
}

#[tokio::test]
async fn uat_style_artifact_creation() {
    // Pattern from uat.toml: create directory structure, write files
    let dir = test_dir();
    let workspace = abs_path(&dir, "workspace/high");
    let manifest = abs_path(&dir, "workspace/manifest.txt");
    let script = format!(
        r#"
        mkdir -p {workspace}
        echo "Test: feature-x" > {manifest}
        echo "Priority: high" >> {manifest}
        cat {manifest}
    "#
    );

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .variable("priority", "high")
        .execute_str(&script)
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(dir.path().join("workspace/high").is_dir());

    let manifest_content =
        std::fs::read_to_string(dir.path().join("workspace/manifest.txt")).unwrap();
    assert!(manifest_content.contains("Test:"));
    assert!(manifest_content.contains("Priority:"));
}

#[tokio::test]
async fn complex_job_with_variables() {
    // Combine variables, pipes, and redirects
    let dir = test_dir();
    let file = abs_path(&dir, "data.txt");
    let result = ShellExecutor::new()
        .cwd(dir.path())
        .variable("PATTERN", "hello")
        .variable("FILE", &file)
        .execute_str(
            r#"
            echo "hello world" > $FILE
            echo "goodbye world" >> $FILE
            cat $FILE | grep $PATTERN
        "#,
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("hello"));
}

#[tokio::test]
async fn heredoc_preserves_content() {
    // Heredocs preserve multi-line content faithfully
    // Note: variable expansion in heredocs is not currently supported;
    // variables are passed through literally
    let result = ShellExecutor::new()
        .execute_str(
            r#"cat << EOF
Hello World
Welcome to the test!
EOF"#,
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let output = result.traces[0].stdout_snippet.as_deref().unwrap();
    assert!(output.contains("Hello World"));
    assert!(output.contains("Welcome to the test!"));
}

#[tokio::test]
async fn heredoc_literal_no_expansion() {
    // Quoted delimiter prevents variable expansion
    let result = ShellExecutor::new()
        .variable("NAME", "Alice")
        .execute_str(
            r#"cat << 'EOF'
Hello $NAME
EOF"#,
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let output = result.traces[0].stdout_snippet.as_deref().unwrap();
    // $NAME should be literal, not expanded
    assert!(output.contains("$NAME"));
}

#[tokio::test]
async fn subshell_with_job() {
    // Pattern: (cmd1 | cmd2)
    let result = ShellExecutor::new()
        .execute_str("(echo hello world | grep hello)")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap()
        .contains("hello"));
}

#[tokio::test]
async fn full_runbook_pattern() {
    // Comprehensive test matching real runbook patterns
    let dir = test_dir();
    let build_dir = abs_path(&dir, "build");
    let output_dir = abs_path(&dir, "build/output");
    let config = abs_path(&dir, "build/config.txt");
    let log = abs_path(&dir, "build/output/log.txt");

    let script = format!(
        r#"
        mkdir -p {output_dir}
        echo "version: 1.0" > {config}
        echo "target: release" >> {config}
        test -f {config} || false
        cat {config} | grep -q "version" || false
        echo "Build started" > {log}
        echo "Configuration loaded" >> {log}
        cat {log}
    "#
    );

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .variable("BUILD_DIR", &build_dir)
        .execute_str(&script)
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);

    // Verify file structure was created
    assert!(dir.path().join("build/output").is_dir());
    assert!(dir.path().join("build/config.txt").exists());
    assert!(dir.path().join("build/output/log.txt").exists());

    // Verify log content
    let log_content = std::fs::read_to_string(dir.path().join("build/output/log.txt")).unwrap();
    assert!(log_content.contains("Build started"));
    assert!(log_content.contains("Configuration loaded"));
}
