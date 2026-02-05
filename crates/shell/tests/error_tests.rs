// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Integration tests for shell execution error handling.
//!
//! These tests verify:
//! - Partial execution tracking (which commands ran before failure)
//! - Error span accuracy (points to correct source location)
//! - Trace completeness (all executed commands recorded even on failure)
//! - Cleanup behavior after mid-script failures

use oj_shell::{diagnostic_context, ExecError, Parser, ShellExecutor};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn executor() -> ShellExecutor {
    ShellExecutor::new()
}

// ---------------------------------------------------------------------------
// Phase 2: Partial Execution Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sequential_stops_at_first_failure() {
    let script = "echo first; false; echo never";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();

    match err {
        ExecError::CommandFailed {
            command,
            exit_code,
            span,
        } => {
            assert_eq!(command, "false");
            assert_eq!(exit_code, 1);
            // Span should point to the and_or_list containing "false" (bytes 12..17)
            assert_eq!(span.start, 12);
            assert_eq!(span.end, 17);
        }
        other => panic!("Expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn job_exit_code_from_last_command() {
    // Job: all stages run concurrently, exit code from last command
    // `echo hello | cat | false | cat` - last command is `cat` which succeeds
    let script = "echo hello | cat | false | cat";
    let result = executor().execute_str(script).await.unwrap();

    // Job succeeds because last command (cat) exits 0
    // Even though `false` failed, job exit is from final stage
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 4);

    // Verify the false command did fail
    assert_eq!(result.traces[2].command, "false");
    assert_eq!(result.traces[2].exit_code, 1);
}

#[tokio::test]
async fn job_fails_when_last_stage_fails() {
    // Job exit code is from the last command
    let script = "true | false";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();

    match err {
        ExecError::CommandFailed { exit_code, .. } => {
            assert_eq!(exit_code, 1);
        }
        other => panic!("Expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn and_chain_short_circuits_on_failure() {
    // `true && echo ran && false && echo skipped`
    // Should execute: true, echo ran, false
    // Should NOT execute: echo skipped
    let script = "true && echo ran && false && echo skipped";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();

    match err {
        ExecError::CommandFailed {
            command, exit_code, ..
        } => {
            assert_eq!(command, "false");
            assert_eq!(exit_code, 1);
        }
        other => panic!("Expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn or_chain_fallback_execution() {
    // `false || echo fallback || true`
    // false fails, triggers OR, echo fallback runs (success), skips true
    let result = executor()
        .execute_str("false || echo fallback || true")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    // Only false and echo fallback should run (true skipped due to success)
    assert_eq!(result.traces.len(), 2);
    assert_eq!(result.traces[0].command, "false");
    assert_eq!(result.traces[1].command, "echo");
}

// ---------------------------------------------------------------------------
// Phase 3: Error Span Accuracy Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn error_span_single_command() {
    let script = "false";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    let span = err.span();

    assert_eq!(span.start, 0);
    assert_eq!(span.end, 5);
}

#[tokio::test]
async fn error_span_second_command_in_sequence() {
    let script = "true; false";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    let span = err.span();

    // "false" starts at byte 6
    let failing_text = &script[span.start..span.end];
    assert_eq!(failing_text, "false");
}

#[tokio::test]
async fn error_span_failing_command_in_and_chain() {
    let script = "true && false && true";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    let span = err.span();

    // Span covers the entire and_or_list
    // The failing command is "false"
    let failing_text = span.slice(script);
    assert!(failing_text.contains("false"));
}

#[tokio::test]
async fn error_span_job_last_command_failure() {
    let script = "echo hi | false";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    let span = err.span();

    // Span should cover the job
    assert!(span.start <= script.find("echo").unwrap());
}

#[tokio::test]
async fn spawn_error_span_covers_command() {
    let script = "nonexistent_command_xyz123";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();

    match err {
        ExecError::SpawnFailed { command, span, .. } => {
            assert_eq!(command, "nonexistent_command_xyz123");
            assert_eq!(span.start, 0);
            assert_eq!(span.end, script.len());
        }
        other => panic!("Expected SpawnFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn redirect_failure_span() {
    let script = "echo test > /nonexistent_path_xyz/file.txt";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();

    match err {
        ExecError::RedirectFailed { span, message, .. } => {
            // Span points to the redirection target
            // Verify the error message references the path
            assert!(message.contains("/nonexistent_path_xyz/file.txt") || span.len() > 0);
        }
        other => panic!("Expected RedirectFailed, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Phase 4: Trace Completeness Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn traces_capture_all_successful_commands() {
    let script = "echo a && echo b && echo c";
    let result = executor().execute_str(script).await.unwrap();

    assert_eq!(result.traces.len(), 3);
    assert_eq!(result.traces[0].command, "echo");
    assert_eq!(result.traces[0].args, vec!["a"]);
    assert_eq!(result.traces[1].args, vec!["b"]);
    assert_eq!(result.traces[2].args, vec!["c"]);

    // All exit codes should be 0
    for trace in &result.traces {
        assert_eq!(trace.exit_code, 0);
        assert!(trace.duration.as_nanos() > 0);
    }
}

#[tokio::test]
async fn job_traces_all_stages() {
    let script = "echo hello | cat | cat";
    let result = executor().execute_str(script).await.unwrap();

    assert_eq!(result.traces.len(), 3);
    assert_eq!(result.traces[0].command, "echo");
    assert_eq!(result.traces[1].command, "cat");
    assert_eq!(result.traces[2].command, "cat");
}

#[tokio::test]
async fn subshell_traces_internal_commands() {
    let script = "(echo inner; echo more)";
    let result = executor().execute_str(script).await.unwrap();

    // Subshell should include traces from inner commands
    assert!(result.traces.len() >= 2);
}

#[tokio::test]
async fn command_substitution_outer_traced() {
    let script = "echo $(echo nested)";
    let result = executor().execute_str(script).await.unwrap();

    // At minimum, the outer echo is traced
    assert!(!result.traces.is_empty());
    assert_eq!(result.traces[0].command, "echo");
}

#[tokio::test]
async fn timing_recorded_for_all_commands() {
    let script = "true && true && true";
    let result = executor().execute_str(script).await.unwrap();

    assert_eq!(result.traces.len(), 3);
    for trace in &result.traces {
        assert!(trace.duration.as_nanos() > 0);
    }
}

// ---------------------------------------------------------------------------
// Phase 5: Cleanup and Resource Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn redirection_file_created_before_failure() {
    let dir = std::env::temp_dir().join("oj_shell_error_test_redir_before_fail");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.txt");

    let script = format!("echo test > {}; false", file.display());
    let err = executor().execute_str(&script).await.unwrap_err();
    assert!(matches!(err, ExecError::CommandFailed { .. }));

    // Verify file was created despite later failure
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content.trim(), "test");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn partial_writes_preserved_on_failure() {
    let dir = std::env::temp_dir().join("oj_shell_error_test_partial");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.txt");

    let script = format!(
        "echo line1 >> {f}; echo line2 >> {f}; false; echo line3 >> {f}",
        f = file.display()
    );

    let err = executor().execute_str(&script).await.unwrap_err();
    assert!(matches!(err, ExecError::CommandFailed { .. }));

    // Verify partial writes were preserved
    let content = std::fs::read_to_string(&file).unwrap();
    assert!(content.contains("line1"));
    assert!(content.contains("line2"));
    assert!(!content.contains("line3"));

    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------------------------------------------------------------------------
// Phase 6: Diagnostic Context Integration Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn diagnostic_context_shows_correct_location() {
    let script = "echo hello\nfalse\necho world";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    let span = err.span();

    let diagnostic = diagnostic_context(script, span, "command failed");

    // Should show line with "false" and caret
    assert!(diagnostic.contains("false"));
    assert!(diagnostic.contains("^"));
    assert!(diagnostic.contains("command failed"));
}

#[tokio::test]
async fn diagnostic_context_multiline_script() {
    let script = "echo one\necho two\nfalse\necho four";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    let span = err.span();

    let diagnostic = diagnostic_context(script, span, "execution failed");

    // Should reference line 3
    assert!(diagnostic.contains("line 3"));
    assert!(diagnostic.contains("false"));
}

#[tokio::test]
async fn span_slice_extracts_failing_text() {
    let script = "true; false; true";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();
    let span = err.span();

    // Use Span::slice to extract the failing text
    let failing_text = span.slice(script);
    assert_eq!(failing_text, "false");
}

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nested_subshell_failure() {
    // Nested subshell: inner failure should propagate
    let script = "((false))";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();

    match err {
        ExecError::CommandFailed { exit_code, .. } => {
            assert_eq!(exit_code, 1);
        }
        other => panic!("Expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn brace_group_failure() {
    let script = "{ false; }";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();

    match err {
        ExecError::CommandFailed { exit_code, .. } => {
            assert_eq!(exit_code, 1);
        }
        other => panic!("Expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn complex_and_or_chain() {
    // true && false || echo recovered
    // - true succeeds, continues to false
    // - false fails, triggers OR, echo runs
    let result = executor()
        .execute_str("true && false || echo recovered")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(result.traces.len() >= 3);
}

#[tokio::test]
async fn multiple_sequential_failures() {
    // Only first failure should be reported (fail-fast)
    let script = "false; false; false";
    let ast = Parser::parse(script).unwrap();
    let err = executor().execute(&ast).await.unwrap_err();

    match err {
        ExecError::CommandFailed { span, .. } => {
            // Span should point to first false
            assert_eq!(span.start, 0);
            assert_eq!(span.end, 5);
        }
        other => panic!("Expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn empty_job_succeeds() {
    // Edge case: script that produces empty job (unlikely in practice)
    // Test that basic success path works
    let result = executor().execute_str("true").await.unwrap();
    assert_eq!(result.exit_code, 0);
}
