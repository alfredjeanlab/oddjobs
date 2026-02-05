//! Init step idempotency specs
//!
//! Verify that the init → reinit recovery pattern works correctly:
//! init failures route to reinit, stale state is cleaned up, concurrent
//! and sequential jobs get isolated workspaces, and the circuit
//! breaker stops infinite init↔reinit cycling.

use crate::prelude::*;

// =============================================================================
// Runbooks
// =============================================================================

/// Job where init always fails, falling back to reinit.
/// init (exit 1) → on_fail → reinit (succeeds) → on_done → work → completes.
const INIT_REINIT_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]
workspace = "folder"

[[job.test.step]]
name = "init"
run = "exit 1"
on_fail = "reinit"
on_done = "work"

[[job.test.step]]
name = "reinit"
run = "echo reinit-ok"
on_done = "work"

[[job.test.step]]
name = "work"
run = "echo done"
"#;

/// Job where init leaves stale state (collision marker) before failing.
/// reinit cleans the marker, then work verifies the workspace is clean.
const STALE_STATE_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]
workspace = "folder"

[[job.test.step]]
name = "init"
run = "touch collision-marker && exit 1"
on_fail = "reinit"
on_done = "work"

[[job.test.step]]
name = "reinit"
run = "rm -f collision-marker"
on_done = "work"

[[job.test.step]]
name = "work"
run = "test ! -f collision-marker"
"#;

/// Job that writes a per-invocation marker for workspace isolation testing.
const ISOLATION_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]
workspace = "folder"

[[job.test.step]]
name = "init"
run = "echo ${var.name} > name.txt"
on_done = "work"

[[job.test.step]]
name = "work"
run = "test -f name.txt"
"#;

/// Job where both init and reinit always fail, creating a cycle.
/// The circuit breaker (MAX_STEP_VISITS = 5) should stop the loop.
const CYCLE_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
vars = ["name"]
workspace = "folder"

[[job.test.step]]
name = "init"
run = "exit 1"
on_fail = "reinit"

[[job.test.step]]
name = "reinit"
run = "exit 1"
on_fail = "init"
"#;

// =============================================================================
// Tests
// =============================================================================

/// When init fails, on_fail routes to reinit, and the job completes.
#[test]
fn init_on_fail_routes_to_reinit_step() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", INIT_REINIT_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test", "reinit-test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should complete via reinit after init failure");
}

/// Reinit cleans up stale state left by a failed init step.
///
/// init creates a collision-marker file then exits non-zero.
/// reinit removes the marker. work verifies the marker is gone.
#[test]
fn reinit_cleans_stale_state_and_recovers() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", STALE_STATE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test", "stale-test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(done, "job should complete after reinit cleans stale state");
}

/// Two concurrent jobs of the same kind get isolated workspaces.
#[test]
fn concurrent_jobs_get_isolated_workspaces() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", ISOLATION_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Launch two jobs with different names
    temp.oj().args(&["run", "test", "alpha"]).passes();
    temp.oj().args(&["run", "test", "beta"]).passes();

    // Both should complete
    let both_done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.matches("completed").count() >= 2
    });

    if !both_done {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        eprintln!("=== JOB LIST ===\n{}\n=== END ===", out);
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        both_done,
        "both jobs should complete with isolated workspaces"
    );
}

/// init↔reinit cycle triggers circuit breaker and job fails.
///
/// Both init and reinit always exit non-zero, routing on_fail to each
/// other indefinitely. The circuit breaker (MAX_STEP_VISITS = 5) stops
/// the loop and fails the job terminally.
#[test]
fn init_reinit_cycle_triggers_circuit_breaker() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", CYCLE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test", "cycle-test"]).passes();

    // Circuit breaker fires after MAX_STEP_VISITS (5) — job fails
    let failed = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("failed")
    });

    if !failed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        failed,
        "job should fail after circuit breaker stops init/reinit cycle"
    );
}

/// Sequential job runs of the same kind don't collide.
///
/// Each job gets a unique workspace ID (based on the job's
/// nonce), so a second run after the first completes should succeed
/// even though both use workspace = "folder".
#[test]
fn sequential_runs_complete_without_workspace_collision() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/test.toml", INIT_REINIT_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // First run
    temp.oj().args(&["run", "test", "first"]).passes();
    let first_done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(first_done, "first job should complete");

    // Second run (same kind, different name)
    temp.oj().args(&["run", "test", "second"]).passes();
    let second_done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.matches("completed").count() >= 2
    });

    if !second_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        second_done,
        "second job should complete without collision from first"
    );
}
