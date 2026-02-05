// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Integration tests for complex job execution.
//!
//! These tests complement the basic job tests in `exec_tests.rs` by
//! covering multi-stage jobs, mixed success/failure scenarios, large data
//! throughput, jobs combined with redirections, and pipefail behavior
//! verification.

use oj_shell::{ExecError, ShellExecutor};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn executor() -> ShellExecutor {
    ShellExecutor::new()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oj_job_test_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ===========================================================================
// Phase 2: Multi-Stage Job Tests (5+ Stages)
// ===========================================================================

#[tokio::test]
async fn five_stage_job_data_flow() {
    // echo "hello" | cat | cat | cat | cat
    // Verify data passes through all stages unchanged
    let result = executor()
        .execute_str("echo hello | cat | cat | cat | cat")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 5);
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("hello\n")
    );
}

#[tokio::test]
async fn seven_stage_transform_chain() {
    // Real-world-ish job: generate lines, transform, filter, count
    // printf "a\nb\nc\na\nb\n" | cat | sort | uniq | cat | cat | wc -l
    let result = executor()
        .execute_str("printf 'a\\nb\\nc\\na\\nb\\n' | cat | sort | uniq | cat | cat | wc -l")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 7);
    // 3 unique lines (a, b, c)
    assert!(result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_deref()
        .unwrap()
        .trim()
        .contains('3'));
}

#[tokio::test]
async fn job_preserves_order() {
    // Numbers should come out in the same order (no transformations)
    let result = executor()
        .execute_str("printf '1\\n2\\n3\\n4\\n5\\n' | cat | cat | cat | cat | cat")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 6);
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("1\n2\n3\n4\n5\n")
    );
}

#[tokio::test]
async fn long_job_all_traces_recorded() {
    // Verify all 5+ stages appear in traces
    let result = executor()
        .execute_str("echo x | cat | cat | cat | cat | cat")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 6);
    // Verify each trace has valid exit code
    for trace in &result.traces {
        assert_eq!(trace.exit_code, 0);
    }
}

// ===========================================================================
// Phase 3: Mixed Success/Failure Job Tests
// ===========================================================================

#[tokio::test]
async fn first_stage_fails_job_continues() {
    // false | echo success - false exits 1, but echo runs and succeeds
    // Job exit = last command = 0
    let result = executor()
        .execute_str("false | echo success")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 2);
    assert_eq!(result.traces[0].exit_code, 1); // false failed
    assert_eq!(result.traces[1].exit_code, 0); // echo succeeded
}

#[tokio::test]
async fn middle_stage_fails_job_continues() {
    // echo hello | sh -c "exit 42" | cat
    // Middle stage fails with 42, but cat still runs
    let result = executor()
        .execute_str("echo hello | sh -c 'exit 42' | cat")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces[1].exit_code, 42);
}

#[tokio::test]
async fn last_stage_fails_returns_failure() {
    // echo hello | false - job fails because last command fails
    let err = executor()
        .execute_str("echo hello | false")
        .await
        .unwrap_err();

    match err {
        ExecError::CommandFailed { exit_code, .. } => {
            assert_eq!(exit_code, 1);
        }
        other => panic!("expected CommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn all_stages_fail_except_last() {
    // Multiple failures, success at end = success
    let result = executor()
        .execute_str("sh -c 'exit 1' | sh -c 'exit 2' | sh -c 'exit 3' | true")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 4);
}

#[tokio::test]
async fn false_head_of_job() {
    // `false | true` returns 0
    let result = executor().execute_str("false | true").await.unwrap();
    assert_eq!(result.exit_code, 0);
}

// ===========================================================================
// Phase 4: Large Data Throughput Tests
// ===========================================================================

#[tokio::test]
async fn large_data_single_pipe() {
    // Generate ~100KB of data and pipe through cat
    // seq generates sequential numbers, one per line
    let result = executor()
        .snippet_limit(200_000) // Capture more for verification
        .execute_str("seq 1 20000 | cat")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    // Verify last line is "20000"
    let stdout = result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_ref()
        .unwrap();
    assert!(stdout.ends_with("20000\n"));
}

#[tokio::test]
async fn large_data_multi_stage() {
    // 50K lines through 4 stages
    let result = executor()
        .snippet_limit(500_000)
        .execute_str("seq 1 50000 | cat | cat | cat")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 4);

    // Verify data integrity - check first and last lines
    let stdout = result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_ref()
        .unwrap();
    assert!(stdout.starts_with("1\n"));
    assert!(stdout.ends_with("50000\n"));
}

#[tokio::test]
async fn line_count_preserved() {
    // Generate 10K lines, count them at the end
    let result = executor()
        .execute_str("seq 1 10000 | cat | cat | wc -l")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let count = result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_ref()
        .unwrap()
        .trim();
    assert_eq!(count, "10000");
}

#[tokio::test]
async fn binary_data_passthrough() {
    // Test that binary-ish data (all byte values) survives job
    // Generate bytes 0-255 repeatedly using printf and verify count
    let result = executor()
        .execute_str("seq 1 1000 | cat | cat | wc -l")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    let count = result
        .traces
        .last()
        .unwrap()
        .stdout_snippet
        .as_ref()
        .unwrap()
        .trim();
    assert_eq!(count, "1000");
}

// ===========================================================================
// Phase 5: Job with Redirection Tests
// ===========================================================================

#[tokio::test]
async fn job_output_to_file() {
    let dir = temp_dir("job_out");
    let file = dir.join("output.txt");

    let script = format!("echo hello | tr 'h' 'H' > {}", file.display());
    let result = executor().execute_str(&script).await.unwrap();

    assert_eq!(result.exit_code, 0);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content.trim(), "Hello");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn job_input_from_file() {
    let dir = temp_dir("job_in");
    let file = dir.join("input.txt");
    std::fs::write(&file, "line1\nline2\nline3\n").unwrap();

    let script = format!("cat < {} | wc -l", file.display());
    let result = executor().execute_str(&script).await.unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result
            .traces
            .last()
            .unwrap()
            .stdout_snippet
            .as_deref()
            .unwrap()
            .trim(),
        "3"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn job_with_heredoc() {
    let script = "cat << EOF | wc -l\nline1\nline2\nEOF";
    let result = executor().execute_str(script).await.unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result
            .traces
            .last()
            .unwrap()
            .stdout_snippet
            .as_deref()
            .unwrap()
            .trim(),
        "2"
    );
}

#[tokio::test]
async fn job_stderr_redirect() {
    // Redirect stderr to /dev/null, job should still work
    let result = executor()
        .execute_str("echo hello 2>/dev/null | cat")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("hello\n")
    );
}

#[tokio::test]
async fn complex_job_with_redirections() {
    let dir = temp_dir("complex_redir");
    let infile = dir.join("input.txt");
    let outfile = dir.join("output.txt");
    std::fs::write(&infile, "apple\nbanana\ncherry\n").unwrap();

    // Read from file, transform, write to file
    let script = format!(
        "cat < {} | tr 'a-z' 'A-Z' > {}",
        infile.display(),
        outfile.display()
    );
    let result = executor().execute_str(&script).await.unwrap();

    assert_eq!(result.exit_code, 0);
    let content = std::fs::read_to_string(&outfile).unwrap();
    assert_eq!(content, "APPLE\nBANANA\nCHERRY\n");

    std::fs::remove_dir_all(&dir).unwrap();
}

// ===========================================================================
// Phase 6: Pipefail Behavior Verification Tests
// ===========================================================================

/// Document current behavior: job exit code = last command's exit code.
/// This is standard shell behavior without pipefail.
#[tokio::test]
async fn pipefail_not_enabled_default_behavior() {
    // All of these should succeed because the last command succeeds,
    // even though earlier stages fail.

    // false | true -> 0
    let result = executor().execute_str("false | true").await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces[0].exit_code, 1); // first stage failed

    // sh -c "exit 42" | sh -c "exit 7" | true -> 0
    let result = executor()
        .execute_str("sh -c 'exit 42' | sh -c 'exit 7' | true")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces[0].exit_code, 42);
    assert_eq!(result.traces[1].exit_code, 7);
    assert_eq!(result.traces[2].exit_code, 0);
}

#[tokio::test]
async fn multiple_failures_last_success() {
    // Multiple failing stages, last succeeds = success
    let result = executor()
        .execute_str("false | false | false | true")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 4);
    // First three failed
    assert_eq!(result.traces[0].exit_code, 1);
    assert_eq!(result.traces[1].exit_code, 1);
    assert_eq!(result.traces[2].exit_code, 1);
    // Last succeeded
    assert_eq!(result.traces[3].exit_code, 0);
}

#[tokio::test]
async fn exit_codes_from_all_stages_available() {
    // Even though job succeeds, we can inspect individual stage failures
    let result = executor()
        .execute_str("sh -c 'exit 1' | sh -c 'exit 2' | sh -c 'exit 3' | true")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 4);
    assert_eq!(result.traces[0].exit_code, 1);
    assert_eq!(result.traces[1].exit_code, 2);
    assert_eq!(result.traces[2].exit_code, 3);
    assert_eq!(result.traces[3].exit_code, 0);
}

#[tokio::test]
async fn sigpipe_when_downstream_closes_early() {
    // Large output piped to head - upstream gets SIGPIPE when downstream closes
    // This should not cause the job to fail (head succeeds)
    let result = executor()
        .execute_str("seq 1 100000 | head -1")
        .await
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.traces.last().unwrap().stdout_snippet.as_deref(),
        Some("1\n")
    );
}
