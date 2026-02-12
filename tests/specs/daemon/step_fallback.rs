//! Job step on_fail fallback and retry cycling specs
//!
//! Verify that step-level and job-level on_fail handlers work correctly,
//! including precedence rules and circuit breaker behavior on retry cycles.

use crate::prelude::*;

/// Step fails → step on_fail routes to "recover" → cleanup runs → job fails.
const STEP_ON_FAIL_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]

[[job.test.step]]
name = "work"
run = "exit 1"
on_fail = { step = "recover" }

[[job.test.step]]
name = "recover"
run = "echo recovered"
"#;

#[test]
fn step_on_fail_routes_to_recovery_step() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", STEP_ON_FAIL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "fallback1"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("failed")
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should fail after step on_fail cleanup completes");

    // Verify job show reveals the fallback step was executed
    let list_output = temp.oj().args(&["job", "list"]).passes().stdout();
    let id = list_output
        .lines()
        .find(|l| l.contains("fallback1"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find job ID");

    let show = temp.oj().args(&["job", "show", id]).passes().stdout();
    assert!(show.contains("recover"), "step history should include the recover step:\n{}", show);
}

/// Step fails (no step on_fail) → job on_fail routes to "cleanup" → cleanup runs → job fails.
const JOB_ON_FAIL_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]
on_fail = { step = "cleanup" }

[[job.test.step]]
name = "work"
run = "exit 1"

[[job.test.step]]
name = "cleanup"
run = "echo cleaned"
"#;

#[test]
fn job_on_fail_used_as_fallback() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", JOB_ON_FAIL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "fallback2"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("failed")
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should fail after job-level on_fail cleanup completes");

    // Verify cleanup step was reached
    let list_output = temp.oj().args(&["job", "list"]).passes().stdout();
    let id = list_output
        .lines()
        .find(|l| l.contains("fallback2"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find job ID");

    temp.oj().args(&["job", "show", id]).passes().stdout_has("cleanup");
}

/// Both step and job define on_fail; step-level wins.
const PRECEDENCE_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]
on_fail = { step = "job-handler" }

[[job.test.step]]
name = "work"
run = "exit 1"
on_fail = { step = "step-handler" }

[[job.test.step]]
name = "step-handler"
run = "echo step-handler-ran"

[[job.test.step]]
name = "job-handler"
run = "echo job-handler-ran"
"#;

#[test]
fn step_on_fail_takes_precedence_over_job_on_fail() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", PRECEDENCE_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "precedence"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("failed")
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should fail after step-level on_fail cleanup completes");

    // Verify step-handler was used (not job-handler)
    let list_output = temp.oj().args(&["job", "list"]).passes().stdout();
    let id = list_output
        .lines()
        .find(|l| l.contains("precedence"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find job ID");

    let show = temp.oj().args(&["job", "show", id]).passes().stdout();
    assert!(show.contains("step-handler"), "step-level on_fail should be used:\n{}", show);
    assert!(
        !show.contains("job-handler"),
        "job-level on_fail should NOT be reached when step on_fail is defined:\n{}",
        show
    );
}

/// When the job-level on_fail target itself fails, the job
/// terminates instead of looping.
const JOB_ON_FAIL_TERMINAL_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]
on_fail = { step = "cleanup" }

[[job.test.step]]
name = "work"
run = "exit 1"

[[job.test.step]]
name = "cleanup"
run = "exit 1"
"#;

#[test]
fn job_on_fail_target_failing_is_terminal() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", JOB_ON_FAIL_TERMINAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "terminal"]).passes();

    let failed = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("failed")
    });

    if !failed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(failed, "job should terminate when on_fail target itself fails");
}

/// Two steps that each fail and route to the other via on_fail.
/// The circuit breaker should fire after MAX_STEP_VISITS and terminate
/// the job instead of cycling forever.
const CYCLE_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]

[[job.test.step]]
name = "attempt"
run = "exit 1"
on_fail = { step = "retry" }

[[job.test.step]]
name = "retry"
run = "exit 1"
on_fail = { step = "attempt" }
"#;

#[test]
#[serial_test::serial]
fn on_fail_cycle_triggers_circuit_breaker() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", CYCLE_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "cycle"]).passes();

    // Wait for the circuit breaker to fire and become visible.
    // Poll for the error message directly — polling only for "failed" status
    // and then checking the error in a separate call is racy because the
    // error may not be flushed to the WAL yet when the status first flips.
    let done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let list_output = temp.oj().args(&["job", "list"]).passes().stdout();
        let id = list_output
            .lines()
            .find(|l| l.contains("cycle"))
            .and_then(|l| l.split_whitespace().next());
        match id {
            Some(id) => {
                temp.oj().args(&["job", "show", id]).passes().stdout().contains("circuit breaker")
            }
            None => false,
        }
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should fail via circuit breaker, not cycle forever");
}
