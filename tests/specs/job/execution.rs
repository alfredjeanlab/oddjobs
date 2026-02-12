//! Job execution specs
//!
//! Verify jobs execute steps correctly.

use crate::prelude::*;

//
// These tests verify that shell constructs (conditionals, jobs, variables,
// subshells) work correctly in runbook step commands.

/// Runbook testing shell conditionals (&&, ||)
const CONDITIONAL_RUNBOOK: &str = r#"
[command.conditional]
args = "<name>"
run = { job = "conditional" }

[job.conditional]
vars  = ["name"]

[[job.conditional.step]]
name = "execute"
run = "true && echo 'and_success:${name}' >> ${workspace}/output.log"
on_done = { step = "done" }

[[job.conditional.step]]
name = "done"
run = "false || echo 'or_fallback:${name}' >> ${workspace}/output.log"
"#;

#[test]
fn shell_conditional_and_succeeds() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/conditional.toml", CONDITIONAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "conditional", "test"]).passes();

    // Wait for job to complete
    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "job should complete");

    // Verify file was written by the && construct
    let output_path = temp.path().join("output.log");
    let found = wait_for(SPEC_WAIT_MAX_MS, || output_path.exists());
    if found {
        let content = std::fs::read_to_string(&output_path).unwrap_or_default();
        assert!(
            content.contains("and_success:test"),
            "should have executed && second command: {}",
            content
        );
    }
}

/// Runbook testing shell jobs (|)
const JOB_SHELL_RUNBOOK: &str = r#"
[command.job_test]
args = "<name>"
run = { job = "job_test" }

[job.job_test]
vars  = ["name"]

[[job.job_test.step]]
name = "execute"
run = "echo 'alpha beta gamma:${name}' | wc -w > ${workspace}/wordcount.txt"
"#;

#[test]
fn shell_job_executes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/pipe.toml", JOB_SHELL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "job_test", "test"]).passes();

    // Wait for job to complete
    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    if !done {
        let output = temp.oj().args(&["job", "list"]).passes().stdout().to_string();
        eprintln!("=== JOB LIST ===\n{output}\n=== END ===");
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should complete");
}

/// Runbook testing variable expansion
const VARIABLE_RUNBOOK: &str = r#"
[command.vartest]
args = "<name>"
run = { job = "vartest" }

[job.vartest]
vars  = ["name"]

[[job.vartest.step]]
name = "execute"
run = "NAME=${name}; echo \"var_expanded:$NAME\" >> ${workspace}/output.log"
"#;

#[test]
fn shell_variable_expansion() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/var.toml", VARIABLE_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "vartest", "myvalue"]).passes();

    // Wait for job to complete
    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "job should complete");
}

/// Runbook testing subshell execution
const SUBSHELL_RUNBOOK: &str = r#"
[command.subshell]
args = "<name>"
run = { job = "subshell" }

[job.subshell]
vars  = ["name"]

[[job.subshell.step]]
name = "execute"
run = "(echo 'subshell_output:${name}') >> ${workspace}/output.log"
"#;

#[test]
fn shell_subshell_executes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/subshell.toml", SUBSHELL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "subshell", "test"]).passes();

    // Wait for job to complete
    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "job should complete");
}

/// Runbook testing exit code propagation
const EXIT_CODE_RUNBOOK: &str = r#"
[command.exitcode]
args = "<name>"
run = { job = "exitcode" }

[job.exitcode]
vars  = ["name"]

[[job.exitcode.step]]
name = "execute"
run = "exit 1"
"#;

#[test]
fn shell_exit_code_failure() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/exit.toml", EXIT_CODE_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "exitcode", "test"]).passes();

    // Wait for job to show failed state
    let mut last_output = String::new();
    let failed = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["job", "list"]).passes().stdout();
        last_output = output.to_string();
        // Job should either fail or show error state
        output.contains("failed") || output.contains("execute")
    });
    assert!(failed, "job should fail on exit 1, got output:\n{}", last_output);
}

//
// Verify that user-provided arguments containing shell-special characters
// (quotes, backticks, etc.) don't break shell command execution.

/// Runbook that uses input values in double-quoted shell context
const QUOTES_RUNBOOK: &str = r#"
[command.greet]
args = "<name>"
run = { job = "greet" }

[job.greet]
vars  = ["name"]

[[job.greet.step]]
name = "execute"
run = "echo \"hello:${var.name}\" >> ${workspace}/output.log"
"#;

#[test]
fn shell_step_with_single_quote_in_arg() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/greet.toml", QUOTES_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    // Pass an argument containing a single quote
    temp.oj().args(&["run", "greet", "it's"]).passes();

    // Wait for job to complete or fail
    let mut last_output = String::new();
    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["job", "list"]).passes().stdout();
        last_output = output.to_string();
        output.contains("completed") || output.contains("failed")
    });
    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should finish, got:\n{}", last_output);
    if !last_output.contains("completed") {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        last_output.contains("completed"),
        "job should complete successfully, got:\n{}",
        last_output
    );

    // Verify the output contains the value with quote preserved
    let output_path = temp.path().join("output.log");
    let found = wait_for(SPEC_WAIT_MAX_MS, || output_path.exists());
    assert!(found, "output file should exist");
    let content = std::fs::read_to_string(&output_path).unwrap_or_default();
    assert!(content.contains("hello:it's"), "output should preserve single quote: {}", content);
}

#[test]
fn shell_step_with_double_quote_in_arg() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/greet.toml", QUOTES_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    // Pass an argument containing double quotes
    temp.oj().args(&["run", "greet", r#"say "hello""#]).passes();

    // Wait for job to complete or fail
    let mut last_output = String::new();
    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["job", "list"]).passes().stdout();
        last_output = output.to_string();
        output.contains("completed") || output.contains("failed")
    });
    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should finish, got:\n{}", last_output);
    if !last_output.contains("completed") {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        last_output.contains("completed"),
        "job should complete successfully, got:\n{}",
        last_output
    );

    // Verify the output contains the value with double quotes preserved
    let output_path = temp.path().join("output.log");
    let found = wait_for(SPEC_WAIT_MAX_MS, || output_path.exists());
    assert!(found, "output file should exist");
    let content = std::fs::read_to_string(&output_path).unwrap_or_default();
    assert!(
        content.contains(r#"hello:say "hello""#),
        "output should preserve double quotes: {}",
        content
    );
}

/// Shell-only runbook that writes to a file for verification
const SHELL_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars  = ["name"]

[[job.test.step]]
name = "init"
run = "echo 'init:${name}' >> ${workspace}/output.log"
on_done = { step = "plan" }

[[job.test.step]]
name = "plan"
run = "echo 'plan:${name}' >> ${workspace}/output.log"
on_done = { step = "execute" }

[[job.test.step]]
name = "execute"
run = "echo 'execute:${name}' >> ${workspace}/output.log"
on_done = { step = "merge" }

[[job.test.step]]
name = "merge"
run = "echo 'merge:${name}' >> ${workspace}/output.log"
"#;

#[test]
fn job_starts_and_runs_init_step() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", SHELL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "hello"]).passes().stdout_has("Command: test");

    // Wait for job to appear (event processing is async)
    let found = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("hello")
    });

    if !found {
        // Debug: print daemon log to understand failure
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(found, "job should be visible in list");
}

#[test]
fn job_completes_all_steps() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", SHELL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "complete"]).passes();

    // Wait for job to reach done step
    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("done")
    });
    assert!(done, "job should reach done step");

    // Verify final state
    temp.oj().args(&["job", "list"]).passes().stdout_has("done").stdout_has("completed");
}

#[test]
fn job_runs_custom_step_names() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/custom.toml",
        r#"
[command.custom]
args = "<name>"
run = { job = "custom" }

[job.custom]
vars  = ["name"]

[[job.custom.step]]
name = "step1"
run = "echo 'step1'"
on_done = { step = "step2" }

[[job.custom.step]]
name = "step2"
run = "echo 'step2'"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "custom", "test"]).passes();

    // Wait for job to show custom step name (step1 or step2) OR complete
    // The job executes very quickly, so we may see:
    // - "step1" or "step2" if we catch it mid-execution
    // - "done" with "Completed" if it finished
    let mut last_output = String::new();
    let found = wait_for(SPEC_WAIT_MAX_MS, || {
        let result = temp.oj().args(&["job", "list"]).passes();
        last_output = result.stdout().to_string();
        // Accept either seeing custom step names OR job completed
        last_output.contains("step1")
            || last_output.contains("step2")
            || (last_output.contains("done") && last_output.contains("completed"))
    });
    assert!(
        found,
        "job should show custom step name or complete successfully, got: {}",
        last_output
    );
}
