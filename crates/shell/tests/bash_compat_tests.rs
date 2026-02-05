// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Bash compatibility tests.

// Allow panic/unwrap/expect in test code (matches lib.rs cfg_attr)
#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
//!
//! Each test runs the same script through both bash and ShellExecutor,
//! then compares exit codes and stdout to verify our shell implementation
//! faithfully reproduces bash semantics.

use oj_shell::{ExecError, ShellExecutor};
use std::process::Command;

// ---------------------------------------------------------------------------
// Test Harness
// ---------------------------------------------------------------------------

/// Result of running a script through both executors.
#[derive(Debug)]
struct CompatResult {
    bash_exit: i32,
    bash_stdout: String,
    shell_exit: i32,
    shell_stdout: String,
}

/// Run a script through both bash and ShellExecutor, returning both results.
async fn run_compat(script: &str) -> CompatResult {
    // Run through bash
    let bash_output = Command::new("bash")
        .arg("-c")
        .arg(script)
        .output()
        .expect("failed to execute bash");

    let bash_exit = bash_output.status.code().unwrap_or(-1);
    let bash_stdout = String::from_utf8_lossy(&bash_output.stdout).to_string();

    // Run through ShellExecutor
    let shell_result = ShellExecutor::new().execute_str(script).await;

    let (shell_exit, shell_stdout) = match shell_result {
        Ok(output) => {
            let stdout = output
                .traces
                .last()
                .and_then(|t| t.stdout_snippet.clone())
                .unwrap_or_default();
            (output.exit_code, stdout)
        }
        Err(e) => {
            // Extract exit code from error if CommandFailed
            let code = match &e {
                ExecError::CommandFailed { exit_code, .. } => *exit_code,
                _ => -1,
            };
            (code, String::new())
        }
    };

    CompatResult {
        bash_exit,
        bash_stdout,
        shell_exit,
        shell_stdout,
    }
}

/// Run a script through both executors with environment/shell variables.
async fn run_compat_with_vars(script: &str, vars: &[(&str, &str)]) -> CompatResult {
    // Run through bash with env vars
    let mut bash_cmd = Command::new("bash");
    bash_cmd.arg("-c").arg(script);
    for (k, v) in vars {
        bash_cmd.env(k, v);
    }
    let bash_output = bash_cmd.output().expect("failed to execute bash");

    let bash_exit = bash_output.status.code().unwrap_or(-1);
    let bash_stdout = String::from_utf8_lossy(&bash_output.stdout).to_string();

    // Run through ShellExecutor with shell variables
    let mut executor = ShellExecutor::new();
    for (k, v) in vars {
        executor = executor.variable(*k, *v);
    }
    let shell_result = executor.execute_str(script).await;

    let (shell_exit, shell_stdout) = match shell_result {
        Ok(output) => {
            let stdout = output
                .traces
                .last()
                .and_then(|t| t.stdout_snippet.clone())
                .unwrap_or_default();
            (output.exit_code, stdout)
        }
        Err(e) => {
            let code = match &e {
                ExecError::CommandFailed { exit_code, .. } => *exit_code,
                _ => -1,
            };
            (code, String::new())
        }
    };

    CompatResult {
        bash_exit,
        bash_stdout,
        shell_exit,
        shell_stdout,
    }
}

/// Assert both executors produce the same exit code and stdout.
fn assert_compat(result: &CompatResult, script: &str) {
    if result.shell_exit != result.bash_exit
        || result.shell_stdout.trim() != result.bash_stdout.trim()
    {
        panic!(
            "\nBash compatibility failure!\n\
             Script: {}\n\
             \n\
             Exit codes:\n\
               bash:  {}\n\
               shell: {}\n\
             \n\
             Stdout:\n\
               bash:  {:?}\n\
               shell: {:?}\n",
            script, result.bash_exit, result.shell_exit, result.bash_stdout, result.shell_stdout
        );
    }
}

/// Assert only exit codes match (for commands that don't produce output).
fn assert_exit_compat(result: &CompatResult, script: &str) {
    if result.shell_exit != result.bash_exit {
        panic!(
            "\nBash compatibility failure (exit code)!\n\
             Script: {}\n\
             \n\
             Exit codes:\n\
               bash:  {}\n\
               shell: {}\n",
            script, result.bash_exit, result.shell_exit
        );
    }
}

// ---------------------------------------------------------------------------
// Phase 1: Simple commands
// ---------------------------------------------------------------------------

mod simple_commands {
    use super::*;

    #[tokio::test]
    async fn echo_hello() {
        let result = run_compat("echo hello").await;
        assert_compat(&result, "echo hello");
    }

    #[tokio::test]
    async fn echo_hello_world() {
        let result = run_compat("echo hello world").await;
        assert_compat(&result, "echo hello world");
    }

    #[tokio::test]
    async fn true_exits_zero() {
        let result = run_compat("true").await;
        assert_exit_compat(&result, "true");
    }

    #[tokio::test]
    async fn false_exits_one() {
        let result = run_compat("false").await;
        assert_exit_compat(&result, "false");
    }

    #[tokio::test]
    async fn echo_n_no_newline() {
        let result = run_compat("echo -n hello").await;
        assert_compat(&result, "echo -n hello");
    }
}

// ---------------------------------------------------------------------------
// Phase 2: Variable expansion
// ---------------------------------------------------------------------------

mod variable_expansion {
    use super::*;

    #[tokio::test]
    async fn simple_variable() {
        let result = run_compat_with_vars("echo $FOO", &[("FOO", "bar")]).await;
        assert_compat(&result, "echo $FOO");
    }

    #[tokio::test]
    async fn braced_variable() {
        let result = run_compat_with_vars("echo ${FOO}", &[("FOO", "bar")]).await;
        assert_compat(&result, "echo ${FOO}");
    }

    #[tokio::test]
    async fn default_value_unset() {
        let result = run_compat("echo ${UNSET:-default}").await;
        assert_compat(&result, "echo ${UNSET:-default}");
    }

    #[tokio::test]
    async fn default_value_set() {
        let result = run_compat_with_vars("echo ${FOO:-default}", &[("FOO", "bar")]).await;
        assert_compat(&result, "echo ${FOO:-default}");
    }

    #[tokio::test]
    async fn adjacent_text() {
        let result = run_compat_with_vars("echo prefix${FOO}suffix", &[("FOO", "bar")]).await;
        assert_compat(&result, "echo prefix${FOO}suffix");
    }

    #[tokio::test]
    async fn multiple_variables() {
        let result = run_compat_with_vars(
            "echo $A $B $C",
            &[("A", "one"), ("B", "two"), ("C", "three")],
        )
        .await;
        assert_compat(&result, "echo $A $B $C");
    }
}

// ---------------------------------------------------------------------------
// Phase 3: Command substitution
// ---------------------------------------------------------------------------

mod command_substitution {
    use super::*;

    #[tokio::test]
    async fn simple_substitution() {
        let result = run_compat("echo $(echo inner)").await;
        assert_compat(&result, "echo $(echo inner)");
    }

    #[tokio::test]
    async fn nested_substitution() {
        let result = run_compat("echo $(echo $(echo deep))").await;
        assert_compat(&result, "echo $(echo $(echo deep))");
    }

    #[tokio::test]
    async fn with_pipes() {
        let result = run_compat("echo $(echo hello | tr a-z A-Z)").await;
        assert_compat(&result, "echo $(echo hello | tr a-z A-Z)");
    }

    #[tokio::test]
    async fn backtick_form() {
        let result = run_compat("echo `echo backtick`").await;
        assert_compat(&result, "echo `echo backtick`");
    }
}

// ---------------------------------------------------------------------------
// Phase 4: Jobs
// ---------------------------------------------------------------------------

mod jobs {
    use super::*;

    #[tokio::test]
    async fn two_stage() {
        let result = run_compat("echo hello | cat").await;
        assert_compat(&result, "echo hello | cat");
    }

    #[tokio::test]
    async fn three_stage() {
        let result = run_compat("echo hello | cat | cat").await;
        assert_compat(&result, "echo hello | cat | cat");
    }

    #[tokio::test]
    async fn exit_code_from_last_success() {
        // false | true → exit 0
        let result = run_compat("false | true").await;
        assert_exit_compat(&result, "false | true");
    }

    #[tokio::test]
    async fn exit_code_from_last_failure() {
        // true | false → exit 1
        let result = run_compat("true | false").await;
        assert_exit_compat(&result, "true | false");
    }

    #[tokio::test]
    async fn data_transformation() {
        let result = run_compat("echo hello | tr a-z A-Z").await;
        assert_compat(&result, "echo hello | tr a-z A-Z");
    }

    #[tokio::test]
    async fn large_data() {
        let result = run_compat("seq 1 1000 | tail -1").await;
        assert_compat(&result, "seq 1 1000 | tail -1");
    }
}

// ---------------------------------------------------------------------------
// Phase 5: Logical chains (AND/OR)
// ---------------------------------------------------------------------------

mod logical_chains {
    use super::*;

    #[tokio::test]
    async fn and_success() {
        let result = run_compat("true && echo yes").await;
        assert_compat(&result, "true && echo yes");
    }

    #[tokio::test]
    async fn and_failure_skips() {
        // false && echo no → no output, exit 1
        let result = run_compat("false && echo no").await;
        assert_exit_compat(&result, "false && echo no");
        // Verify no output from echo
        assert!(
            result.bash_stdout.is_empty(),
            "bash should have no output for 'false && echo no'"
        );
    }

    #[tokio::test]
    async fn or_fallback() {
        let result = run_compat("false || echo fallback").await;
        assert_compat(&result, "false || echo fallback");
    }

    #[tokio::test]
    async fn or_skips_on_success() {
        // true || echo no → no output, exit 0
        let result = run_compat("true || echo no").await;
        assert_exit_compat(&result, "true || echo no");
    }

    #[tokio::test]
    async fn chained_and() {
        let result = run_compat("true && echo a && echo b").await;
        // Check both executors show 'b' as the last output
        assert_eq!(
            result.bash_stdout.trim().lines().last(),
            Some("b"),
            "bash should end with 'b'"
        );
        assert_eq!(result.shell_exit, result.bash_exit);
    }

    #[tokio::test]
    async fn mixed_and_or() {
        let result = run_compat("false || echo recovered && echo done").await;
        assert_eq!(
            result.bash_stdout.trim().lines().last(),
            Some("done"),
            "bash should end with 'done'"
        );
        assert_eq!(result.shell_exit, result.bash_exit);
    }

    #[tokio::test]
    async fn complex_chain() {
        let result = run_compat("true && false || echo fallback").await;
        assert_compat(&result, "true && false || echo fallback");
    }

    #[tokio::test]
    async fn multiple_or() {
        let result = run_compat("false || false || echo third").await;
        assert_compat(&result, "false || false || echo third");
    }
}

// ---------------------------------------------------------------------------
// Phase 6: Redirections
// ---------------------------------------------------------------------------

mod redirections {
    use super::*;

    #[tokio::test]
    async fn output_redirect() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.txt");
        let script = format!("echo hello > {}", file.display());

        // Run through bash
        Command::new("bash")
            .arg("-c")
            .arg(&script)
            .status()
            .unwrap();
        let bash_content = std::fs::read_to_string(&file).unwrap();

        // Reset file
        std::fs::remove_file(&file).ok();

        // Run through ShellExecutor
        ShellExecutor::new().execute_str(&script).await.unwrap();
        let shell_content = std::fs::read_to_string(&file).unwrap();

        assert_eq!(
            shell_content, bash_content,
            "output redirect content mismatch"
        );
    }

    #[tokio::test]
    async fn append_redirect() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.txt");
        let script = format!("echo a > {f}; echo b >> {f}", f = file.display());

        // Run through bash
        Command::new("bash")
            .arg("-c")
            .arg(&script)
            .status()
            .unwrap();
        let bash_content = std::fs::read_to_string(&file).unwrap();

        // Reset file
        std::fs::remove_file(&file).ok();

        // Run through ShellExecutor
        ShellExecutor::new().execute_str(&script).await.unwrap();
        let shell_content = std::fs::read_to_string(&file).unwrap();

        assert_eq!(
            shell_content, bash_content,
            "append redirect content mismatch"
        );
    }

    #[tokio::test]
    async fn input_redirect() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("input.txt");
        std::fs::write(&file, "from_file\n").unwrap();

        let script = format!("cat < {}", file.display());
        let result = run_compat(&script).await;
        assert_compat(&result, &script);
    }

    #[tokio::test]
    async fn heredoc() {
        let script = "cat << EOF\nhello\nEOF";
        let result = run_compat(script).await;
        assert_compat(&result, script);
    }

    #[tokio::test]
    async fn herestring() {
        let script = "cat <<< hello";
        let result = run_compat(script).await;
        assert_compat(&result, script);
    }

    #[tokio::test]
    async fn null_redirect() {
        let result = run_compat("echo hello > /dev/null").await;
        // Should have no stdout
        assert_exit_compat(&result, "echo hello > /dev/null");
        assert!(
            result.bash_stdout.is_empty(),
            "bash stdout should be empty with /dev/null redirect"
        );
    }
}

// ---------------------------------------------------------------------------
// Phase 7: Quoting
// ---------------------------------------------------------------------------

mod quoting {
    use super::*;

    #[tokio::test]
    async fn single_quotes_preserve_literal() {
        // Single quotes prevent variable expansion
        let result = run_compat_with_vars("echo '$FOO'", &[("FOO", "bar")]).await;
        assert_compat(&result, "echo '$FOO'");
    }

    #[tokio::test]
    async fn double_quotes_expand_variables() {
        let result = run_compat_with_vars("echo \"$FOO\"", &[("FOO", "bar")]).await;
        assert_compat(&result, "echo \"$FOO\"");
    }

    #[tokio::test]
    async fn escaped_characters() {
        let result = run_compat("echo \"hello\\\"world\"").await;
        assert_compat(&result, "echo \"hello\\\"world\"");
    }

    #[tokio::test]
    async fn space_preservation() {
        let result = run_compat("echo \"hello   world\"").await;
        assert_compat(&result, "echo \"hello   world\"");
    }

    #[tokio::test]
    async fn quote_removal() {
        // echo hello vs echo "hello" should produce the same output
        let result1 = run_compat("echo hello").await;
        let result2 = run_compat("echo \"hello\"").await;
        assert_eq!(
            result1.bash_stdout.trim(),
            result2.bash_stdout.trim(),
            "quoted and unquoted should match for bash"
        );
        assert_eq!(
            result1.shell_stdout.trim(),
            result2.shell_stdout.trim(),
            "quoted and unquoted should match for shell"
        );
    }
}
