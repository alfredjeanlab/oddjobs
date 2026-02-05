//! Cron→job integration specs
//!
//! Verify that cron-triggered jobs execute correctly, handle failures
//! gracefully, and that cron lifecycle does not interfere with running jobs.

use crate::prelude::*;

// =============================================================================
// Runbook definitions
// =============================================================================

/// Multi-step job triggered by cron. Steps: prep → work → done.
const MULTI_STEP_CRON_RUNBOOK: &str = r#"
[cron.builder]
interval = "500ms"
run = { job = "build" }

[job.build]

[[job.build.step]]
name = "prep"
run = "echo preparing"
on_done = "work"

[[job.build.step]]
name = "work"
run = "echo building"
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo finished"
"#;

/// Cron with a job that always fails on its first step.
const FAILING_CRON_RUNBOOK: &str = r#"
[cron.breaker]
interval = "500ms"
run = { job = "fail" }

[job.fail]

[[job.fail.step]]
name = "explode"
run = "exit 1"
"#;

/// Cron with a fast single-step job.
const FAST_CRON_RUNBOOK: &str = r#"
[cron.ticker]
interval = "500ms"
run = { job = "tick" }

[job.tick]

[[job.tick.step]]
name = "work"
run = "echo tick"
"#;

/// Cron with a blocking job (sleep 30) for testing stop-while-running.
const SLOW_CRON_RUNBOOK: &str = r#"
[cron.slow]
interval = "60s"
run = { job = "slow" }

[job.slow]

[[job.slow.step]]
name = "blocking"
run = "sleep 30"
"#;

// =============================================================================
// Test 1: Cron-triggered job completes all steps
// =============================================================================

/// Verifies that a multi-step job triggered by `oj cron once` runs all
/// steps to completion (prep → work → done), not just job creation.
#[test]
fn cron_triggered_job_completes_all_steps() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", MULTI_STEP_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Use `cron once` to trigger immediately (no interval wait)
    temp.oj()
        .args(&["cron", "once", "builder"])
        .passes()
        .stdout_has("Job")
        .stdout_has("started");

    // Wait for the job to complete all 3 steps
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        completed,
        "cron-triggered multi-step job should complete all steps"
    );

    // Verify job did not fail
    let list = temp.oj().args(&["job", "list"]).passes().stdout();
    assert!(
        !list.contains("failed"),
        "job should not have failed, got: {}",
        list
    );
}

// =============================================================================
// Test 2: Failed job doesn't stop cron from firing again
// =============================================================================

/// When a cron-triggered job fails, the cron timer should continue
/// firing and create new jobs on subsequent ticks.
#[test]
fn cron_keeps_firing_after_job_failure() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", FAILING_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["cron", "start", "breaker"]).passes();

    // Wait for at least 2 jobs to appear (proving cron fired more than once
    // despite the first job failing immediately)
    let multiple_fires = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let output = temp.oj().args(&["job", "list"]).passes().stdout();
        output.matches("fail").count() >= 2
    });

    if !multiple_fires {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        multiple_fires,
        "cron should keep firing after job failure, creating multiple jobs"
    );

    // Cron should still be running
    temp.oj()
        .args(&["cron", "list"])
        .passes()
        .stdout_has("running");

    temp.oj().args(&["cron", "stop", "breaker"]).passes();
}

// =============================================================================
// Test 3: Multiple cron-once invocations create independent jobs
// =============================================================================

/// Each `oj cron once` invocation should create a distinct job with
/// its own lifecycle, and both should complete independently.
#[test]
fn cron_creates_independent_jobs() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", FAST_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Trigger two independent jobs
    temp.oj().args(&["cron", "once", "ticker"]).passes();
    temp.oj().args(&["cron", "once", "ticker"]).passes();

    // Wait for both to complete
    let both_completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["job", "list"]).passes().stdout();
        output.matches("completed").count() >= 2
    });

    if !both_completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        both_completed,
        "both cron-triggered jobs should complete independently"
    );
}

// =============================================================================
// Test 4: Stopping cron doesn't kill running job
// =============================================================================

/// When a cron is stopped while one of its triggered jobs is still
/// running, the job should continue unaffected.
#[test]
fn cron_stop_does_not_kill_running_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", SLOW_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Trigger the slow job via cron once
    temp.oj().args(&["cron", "once", "slow"]).passes();

    // Wait for job to reach running state
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("running")
    });
    assert!(running, "job should be running");

    // Start and stop the cron — should NOT affect the already-running job
    temp.oj().args(&["cron", "start", "slow"]).passes();
    temp.oj().args(&["cron", "stop", "slow"]).passes();

    // Job should still be running (the sleep 30 hasn't finished)
    let still_running = temp.oj().args(&["job", "list"]).passes().stdout();
    assert!(
        still_running.contains("running"),
        "job should still be running after cron stop, got: {}",
        still_running
    );
}

// =============================================================================
// Test 5: Cron restart picks up runbook changes
// =============================================================================

/// After modifying the runbook and restarting the cron, `oj cron once` should
/// use the updated job definition.
#[test]
fn cron_restart_picks_up_runbook_changes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", FAST_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["cron", "start", "ticker"]).passes();

    // Update the runbook with a different step name
    let updated_runbook = r#"
[cron.ticker]
interval = "2s"
run = { job = "tick" }

[job.tick]

[[job.tick.step]]
name = "updated-work"
run = "echo updated"
"#;
    temp.file(".oj/runbooks/cron.toml", updated_runbook);

    // Restart to pick up the change
    temp.oj()
        .args(&["cron", "restart", "ticker"])
        .passes()
        .stdout_has("restarted");

    // Trigger job with the new definition
    temp.oj().args(&["cron", "once", "ticker"]).passes();

    // Wait for job to complete
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        completed,
        "job with updated runbook should complete after cron restart"
    );

    // Stop the cron timer
    temp.oj().args(&["cron", "stop", "ticker"]).passes();
}
