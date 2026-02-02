// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for variable expansion, environment prefix, modifiers, and special variables.

use super::executor;
use crate::exec::{ExecError, ShellExecutor};

// ---------------------------------------------------------------------------
// Variable expansion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn variable_expansion() {
    let result = ShellExecutor::new()
        .variable("FOO", "bar")
        .execute_str("echo $FOO")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("bar\n"));
}

#[tokio::test]
async fn variable_default() {
    let result = executor().execute_str("echo ${X:-default}").await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("default\n")
    );
}

// ---------------------------------------------------------------------------
// Environment prefix (VAR=value cmd)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn env_prefix_sets_variable_for_command() {
    // VAR=value cmd syntax should set the environment variable for the command
    let result = executor()
        .execute_str("FOO=bar sh -c 'echo $FOO'")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("bar\n")
    );
}

#[tokio::test]
async fn env_prefix_with_command_substitution() {
    // FOO=$(echo bar) cmd should evaluate the command substitution
    let result = executor()
        .execute_str("FOO=$(echo bar) sh -c 'echo $FOO'")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("bar\n")
    );
}

// ---------------------------------------------------------------------------
// Special shell variables ($?, $$, $#, $0)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn special_variable_exit_code() {
    // $? reflects exit code of last command (after true, should be 0)
    let result = executor().execute_str("true && echo $?").await.unwrap();
    assert_eq!(result.traces[1].stdout_snippet.as_deref(), Some("0\n"));
}

#[tokio::test]
async fn special_variable_exit_code_after_or() {
    // After `false || true`, $? should reflect `true` (0)
    let result = executor()
        .execute_str("false || true && echo $?")
        .await
        .unwrap();
    assert_eq!(result.traces[2].stdout_snippet.as_deref(), Some("0\n"));
}

#[tokio::test]
async fn special_variable_pid() {
    // $$ is current process ID
    let result = executor().execute_str("echo $$").await.unwrap();
    let output = result.traces[0].stdout_snippet.as_ref().unwrap().trim();
    let pid: u32 = output.parse().expect("should be valid PID");
    assert_eq!(pid, std::process::id());
}

#[tokio::test]
async fn special_variable_arg_count() {
    // $# is always 0 for oj-shell (no positional arguments)
    let result = executor().execute_str("echo $#").await.unwrap();
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("0\n"));
}

#[tokio::test]
async fn special_variable_script_name() {
    // $0 is "oj-shell"
    let result = executor().execute_str("echo $0").await.unwrap();
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("oj-shell\n")
    );
}

#[tokio::test]
async fn special_variable_with_modifier() {
    // ${?:-default} when $? is 0 should return "0" not "default"
    let result = executor()
        .execute_str("true && echo ${?:-default}")
        .await
        .unwrap();
    assert_eq!(result.traces[1].stdout_snippet.as_deref(), Some("0\n"));
}

#[tokio::test]
async fn special_variable_braced() {
    // ${$} should work same as $$
    let result = executor().execute_str("echo ${$}").await.unwrap();
    let output = result.traces[0].stdout_snippet.as_ref().unwrap().trim();
    let pid: u32 = output.parse().expect("should be valid PID");
    assert_eq!(pid, std::process::id());
}

#[tokio::test]
async fn special_variable_braced_script_name() {
    // ${0} should work same as $0
    let result = executor().execute_str("echo ${0}").await.unwrap();
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("oj-shell\n")
    );
}

#[tokio::test]
async fn special_variable_braced_arg_count() {
    // ${#} should work same as $#
    let result = executor().execute_str("echo ${#}").await.unwrap();
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("0\n"));
}

// ---------------------------------------------------------------------------
// Variable modifier tests (parameterized)
// ---------------------------------------------------------------------------

mod variable_modifiers {
    use super::*;
    use yare::parameterized;

    /// Helper to create executor with optional variable value.
    /// - `None` means unset
    /// - `Some("")` means empty
    /// - `Some("value")` means set
    fn executor_with_var(value: Option<&str>) -> ShellExecutor {
        let mut exec = ShellExecutor::new();
        if let Some(v) = value {
            exec = exec.variable("VAR", v);
        }
        exec
    }

    /// Sync wrapper for async execution
    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    // -------------------------------------------------------------------------
    // :- (use default if unset OR empty)
    // -------------------------------------------------------------------------

    #[parameterized(
        unset = { None, "default" },
        empty = { Some(""), "default" },
        set = { Some("value"), "value" },
    )]
    fn colon_minus(var_value: Option<&str>, expected: &str) {
        run(async {
            let result = executor_with_var(var_value)
                .execute_str("echo ${VAR:-default}")
                .await
                .unwrap();
            let expected_out = format!("{expected}\n");
            assert_eq!(
                result.traces[0].stdout_snippet.as_deref(),
                Some(expected_out.as_str())
            );
        })
    }

    // -------------------------------------------------------------------------
    // - (use default if unset only)
    // -------------------------------------------------------------------------

    #[parameterized(
        unset = { None, "default" },
        empty = { Some(""), "" },
        set = { Some("value"), "value" },
    )]
    fn minus(var_value: Option<&str>, expected: &str) {
        run(async {
            let result = executor_with_var(var_value)
                .execute_str("echo ${VAR-default}")
                .await
                .unwrap();
            let expected_out = format!("{expected}\n");
            assert_eq!(
                result.traces[0].stdout_snippet.as_deref(),
                Some(expected_out.as_str())
            );
        })
    }

    // -------------------------------------------------------------------------
    // := (assign default if unset OR empty)
    // -------------------------------------------------------------------------

    #[parameterized(
        unset = { None, "default" },
        empty = { Some(""), "default" },
        set = { Some("value"), "value" },
    )]
    fn colon_equals(var_value: Option<&str>, expected: &str) {
        run(async {
            let result = executor_with_var(var_value)
                .execute_str("echo ${VAR:=default}")
                .await
                .unwrap();
            let expected_out = format!("{expected}\n");
            assert_eq!(
                result.traces[0].stdout_snippet.as_deref(),
                Some(expected_out.as_str())
            );
        })
    }

    // -------------------------------------------------------------------------
    // = (assign default if unset only)
    // -------------------------------------------------------------------------

    #[parameterized(
        unset = { None, "default" },
        empty = { Some(""), "" },
        set = { Some("value"), "value" },
    )]
    fn equals(var_value: Option<&str>, expected: &str) {
        run(async {
            let result = executor_with_var(var_value)
                .execute_str("echo ${VAR=default}")
                .await
                .unwrap();
            let expected_out = format!("{expected}\n");
            assert_eq!(
                result.traces[0].stdout_snippet.as_deref(),
                Some(expected_out.as_str())
            );
        })
    }

    // -------------------------------------------------------------------------
    // :+ (use alternative if set AND non-empty)
    // -------------------------------------------------------------------------

    #[parameterized(
        unset = { None, "" },
        empty = { Some(""), "" },
        set = { Some("value"), "alt" },
    )]
    fn colon_plus(var_value: Option<&str>, expected: &str) {
        run(async {
            let result = executor_with_var(var_value)
                .execute_str("echo ${VAR:+alt}")
                .await
                .unwrap();
            let expected_out = format!("{expected}\n");
            assert_eq!(
                result.traces[0].stdout_snippet.as_deref(),
                Some(expected_out.as_str())
            );
        })
    }

    // -------------------------------------------------------------------------
    // + (use alternative if set, even if empty)
    // -------------------------------------------------------------------------

    #[parameterized(
        unset = { None, "" },
        empty = { Some(""), "alt" },
        set = { Some("value"), "alt" },
    )]
    fn plus(var_value: Option<&str>, expected: &str) {
        run(async {
            let result = executor_with_var(var_value)
                .execute_str("echo ${VAR+alt}")
                .await
                .unwrap();
            let expected_out = format!("{expected}\n");
            assert_eq!(
                result.traces[0].stdout_snippet.as_deref(),
                Some(expected_out.as_str())
            );
        })
    }

    // -------------------------------------------------------------------------
    // :? (error if unset OR empty)
    // -------------------------------------------------------------------------

    #[parameterized(
        unset = { None, true },
        empty = { Some(""), true },
        set = { Some("value"), false },
    )]
    fn colon_question(var_value: Option<&str>, should_error: bool) {
        run(async {
            let result = executor_with_var(var_value)
                .execute_str("echo ${VAR:?err}")
                .await;
            if should_error {
                match result.unwrap_err() {
                    ExecError::UndefinedVariable { name, .. } => assert_eq!(name, "VAR"),
                    other => panic!("expected UndefinedVariable, got: {other:?}"),
                }
            } else {
                let r = result.unwrap();
                assert_eq!(r.traces[0].stdout_snippet.as_deref(), Some("value\n"));
            }
        })
    }

    // -------------------------------------------------------------------------
    // ? (error if unset only)
    // -------------------------------------------------------------------------

    #[parameterized(
        unset = { None, true },
        empty = { Some(""), false },
        set = { Some("value"), false },
    )]
    fn question(var_value: Option<&str>, should_error: bool) {
        run(async {
            let result = executor_with_var(var_value)
                .execute_str("echo ${VAR?err}")
                .await;
            if should_error {
                match result.unwrap_err() {
                    ExecError::UndefinedVariable { name, .. } => assert_eq!(name, "VAR"),
                    other => panic!("expected UndefinedVariable, got: {other:?}"),
                }
            } else {
                let r = result.unwrap();
                let expected = if var_value == Some("") {
                    "\n"
                } else {
                    "value\n"
                };
                assert_eq!(r.traces[0].stdout_snippet.as_deref(), Some(expected));
            }
        })
    }

    // -------------------------------------------------------------------------
    // Nounset behavior (always on): unset variables without modifiers error
    // -------------------------------------------------------------------------

    #[parameterized(
        simple = { "echo $VAR" },
        braced = { "echo ${VAR}" },
    )]
    fn nounset_unset_errors(script: &str) {
        run(async {
            let result = executor().execute_str(script).await;
            match result.unwrap_err() {
                ExecError::UndefinedVariable { name, .. } => assert_eq!(name, "VAR"),
                other => panic!("expected UndefinedVariable, got: {other:?}"),
            }
        })
    }

    #[parameterized(
        simple = { "echo $VAR", "value\n" },
        braced = { "echo ${VAR}", "value\n" },
        empty = { "echo ${VAR}", "\n" },
    )]
    fn nounset_set_succeeds(script: &str, expected: &str) {
        run(async {
            let value = if expected == "\n" { "" } else { "value" };
            let result = executor()
                .variable("VAR", value)
                .execute_str(script)
                .await
                .unwrap();
            assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some(expected));
        })
    }

    #[parameterized(
        dash = { "echo ${VAR:-}" },
        colon_dash = { "echo ${VAR:-default}" },
        equals = { "echo ${VAR:=default}" },
    )]
    fn nounset_with_modifier_allowed(script: &str) {
        run(async {
            // Should not error - modifiers provide explicit defaults
            let result = executor().execute_str(script).await.unwrap();
            assert_eq!(result.exit_code, 0);
        })
    }

    // -------------------------------------------------------------------------
    // Variable expansion in modifier defaults
    // -------------------------------------------------------------------------

    #[parameterized(
        var = { "echo ${X:-$Y}", Some(("Y", "ok")), None, "ok\n" },
        braced = { "echo ${X:-${Y}}", Some(("Y", "ok")), None, "ok\n" },
        cmd_sub = { "echo ${X:-$(echo ok)}", None, None, "ok\n" },
        alt = { "echo ${X:+$Y}", Some(("Y", "ok")), Some(("X", "set")), "ok\n" },
    )]
    fn modifier_expands_default(
        cmd: &str,
        var: Option<(&str, &str)>,
        var2: Option<(&str, &str)>,
        expected: &str,
    ) {
        run(async {
            let mut exec = executor();
            if let Some((k, v)) = var {
                exec = exec.variable(k, v);
            }
            if let Some((k, v)) = var2 {
                exec = exec.variable(k, v);
            }
            let result = exec.execute_str(cmd).await.unwrap();
            let last = result.traces.last().unwrap();
            assert_eq!(last.stdout_snippet.as_deref(), Some(expected));
        })
    }
}
