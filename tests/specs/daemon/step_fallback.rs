//! Pipeline step on_fail fallback and retry cycling specs
//!
//! Verify that step-level and pipeline-level on_fail handlers work correctly,
//! including precedence rules and circuit breaker behavior on retry cycles.

use crate::prelude::*;

// =============================================================================
// Test 1: Step-level on_fail routes to recovery step
// =============================================================================

/// Step fails → step on_fail routes to "recover" → pipeline completes.
const STEP_ON_FAIL_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { pipeline = "test" }

[pipeline.test]
vars = ["name"]

[[pipeline.test.step]]
name = "work"
run = "exit 1"
on_fail = "recover"

[[pipeline.test.step]]
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
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        done,
        "pipeline should complete after step on_fail routes to recover"
    );

    // Verify pipeline show reveals the fallback step was executed
    let list_output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    let id = list_output
        .lines()
        .find(|l| l.contains("fallback1"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find pipeline ID");

    let show = temp.oj().args(&["pipeline", "show", id]).passes().stdout();
    assert!(
        show.contains("recover"),
        "step history should include the recover step:\n{}",
        show
    );
}

// =============================================================================
// Test 2: Pipeline-level on_fail used as fallback
// =============================================================================

/// Step fails (no step on_fail) → pipeline on_fail routes to "cleanup" → completes.
const PIPELINE_ON_FAIL_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { pipeline = "test" }

[pipeline.test]
vars = ["name"]
on_fail = "cleanup"

[[pipeline.test.step]]
name = "work"
run = "exit 1"

[[pipeline.test.step]]
name = "cleanup"
run = "echo cleaned"
"#;

#[test]
fn pipeline_on_fail_used_as_fallback() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", PIPELINE_ON_FAIL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "fallback2"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        done,
        "pipeline should complete via pipeline-level on_fail fallback"
    );

    // Verify cleanup step was reached
    let list_output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    let id = list_output
        .lines()
        .find(|l| l.contains("fallback2"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find pipeline ID");

    temp.oj()
        .args(&["pipeline", "show", id])
        .passes()
        .stdout_has("cleanup");
}

// =============================================================================
// Test 3: Step on_fail takes precedence over pipeline on_fail
// =============================================================================

/// Both step and pipeline define on_fail; step-level wins.
const PRECEDENCE_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { pipeline = "test" }

[pipeline.test]
vars = ["name"]
on_fail = "pipeline-handler"

[[pipeline.test.step]]
name = "work"
run = "exit 1"
on_fail = "step-handler"

[[pipeline.test.step]]
name = "step-handler"
run = "echo step-handler-ran"

[[pipeline.test.step]]
name = "pipeline-handler"
run = "echo pipeline-handler-ran"
"#;

#[test]
fn step_on_fail_takes_precedence_over_pipeline_on_fail() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", PRECEDENCE_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "precedence"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "pipeline should complete via step-level on_fail");

    // Verify step-handler was used (not pipeline-handler)
    let list_output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    let id = list_output
        .lines()
        .find(|l| l.contains("precedence"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find pipeline ID");

    let show = temp.oj().args(&["pipeline", "show", id]).passes().stdout();
    assert!(
        show.contains("step-handler"),
        "step-level on_fail should be used:\n{}",
        show
    );
    assert!(
        !show.contains("pipeline-handler"),
        "pipeline-level on_fail should NOT be reached when step on_fail is defined:\n{}",
        show
    );
}

// =============================================================================
// Test 4: Pipeline on_fail target failing = terminal failure
// =============================================================================

/// When the pipeline-level on_fail target itself fails, the pipeline
/// terminates instead of looping.
const PIPELINE_ON_FAIL_TERMINAL_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { pipeline = "test" }

[pipeline.test]
vars = ["name"]
on_fail = "cleanup"

[[pipeline.test.step]]
name = "work"
run = "exit 1"

[[pipeline.test.step]]
name = "cleanup"
run = "exit 1"
"#;

#[test]
fn pipeline_on_fail_target_failing_is_terminal() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", PIPELINE_ON_FAIL_TERMINAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "terminal"]).passes();

    let failed = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("failed")
    });

    if !failed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        failed,
        "pipeline should terminate when on_fail target itself fails"
    );
}

// =============================================================================
// Test 5: on_fail cycle triggers circuit breaker
// =============================================================================

/// Two steps that each fail and route to the other via on_fail.
/// The circuit breaker should fire after MAX_STEP_VISITS and terminate
/// the pipeline instead of cycling forever.
const CYCLE_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { pipeline = "test" }

[pipeline.test]
vars = ["name"]

[[pipeline.test.step]]
name = "attempt"
run = "exit 1"
on_fail = "retry"

[[pipeline.test.step]]
name = "retry"
run = "exit 1"
on_fail = "attempt"
"#;

#[test]
fn on_fail_cycle_triggers_circuit_breaker() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", CYCLE_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj().args(&["run", "test", "cycle"]).passes();

    // The circuit breaker should fire after several rounds through the cycle
    let failed = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("failed")
    });

    if !failed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        failed,
        "pipeline should fail via circuit breaker, not cycle forever"
    );

    // Verify the error mentions the circuit breaker
    let list_output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    let id = list_output
        .lines()
        .find(|l| l.contains("cycle"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find pipeline ID");

    temp.oj()
        .args(&["pipeline", "show", id])
        .passes()
        .stdout_has("circuit breaker");
}
