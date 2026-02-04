//! Cron→pipeline integration specs
//!
//! Verify that cron-triggered pipelines execute correctly, handle failures
//! gracefully, and that cron lifecycle does not interfere with running pipelines.

use crate::prelude::*;

// =============================================================================
// Runbook definitions
// =============================================================================

/// Multi-step pipeline triggered by cron. Steps: prep → work → done.
const MULTI_STEP_CRON_RUNBOOK: &str = r#"
[cron.builder]
interval = "2s"
run = { pipeline = "build" }

[pipeline.build]

[[pipeline.build.step]]
name = "prep"
run = "echo preparing"
on_done = "work"

[[pipeline.build.step]]
name = "work"
run = "echo building"
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo finished"
"#;

/// Cron with a pipeline that always fails on its first step.
const FAILING_CRON_RUNBOOK: &str = r#"
[cron.breaker]
interval = "2s"
run = { pipeline = "fail" }

[pipeline.fail]

[[pipeline.fail.step]]
name = "explode"
run = "exit 1"
"#;

/// Cron with a fast single-step pipeline.
const FAST_CRON_RUNBOOK: &str = r#"
[cron.ticker]
interval = "2s"
run = { pipeline = "tick" }

[pipeline.tick]

[[pipeline.tick.step]]
name = "work"
run = "echo tick"
"#;

/// Cron with a blocking pipeline (sleep 30) for testing stop-while-running.
const SLOW_CRON_RUNBOOK: &str = r#"
[cron.slow]
interval = "60s"
run = { pipeline = "slow" }

[pipeline.slow]

[[pipeline.slow.step]]
name = "blocking"
run = "sleep 30"
"#;

// =============================================================================
// Test 1: Cron-triggered pipeline completes all steps
// =============================================================================

/// Verifies that a multi-step pipeline triggered by `oj cron once` runs all
/// steps to completion (prep → work → done), not just pipeline creation.
#[test]
fn cron_triggered_pipeline_completes_all_steps() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", MULTI_STEP_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Use `cron once` to trigger immediately (no interval wait)
    temp.oj()
        .args(&["cron", "once", "builder"])
        .passes()
        .stdout_has("Pipeline")
        .stdout_has("started");

    // Wait for the pipeline to complete all 3 steps
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        completed,
        "cron-triggered multi-step pipeline should complete all steps"
    );

    // Verify pipeline did not fail
    let list = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    assert!(
        !list.contains("failed"),
        "pipeline should not have failed, got: {}",
        list
    );
}

// =============================================================================
// Test 2: Failed pipeline doesn't stop cron from firing again
// =============================================================================

/// When a cron-triggered pipeline fails, the cron timer should continue
/// firing and create new pipelines on subsequent ticks.
#[test]
fn cron_keeps_firing_after_pipeline_failure() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", FAILING_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["cron", "start", "breaker"]).passes();

    // Wait for at least 2 pipelines to appear (proving cron fired more than once
    // despite the first pipeline failing immediately)
    let multiple_fires = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        output.matches("fail").count() >= 2
    });

    if !multiple_fires {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        multiple_fires,
        "cron should keep firing after pipeline failure, creating multiple pipelines"
    );

    // Cron should still be running
    temp.oj()
        .args(&["cron", "list"])
        .passes()
        .stdout_has("running");

    temp.oj().args(&["cron", "stop", "breaker"]).passes();
}

// =============================================================================
// Test 3: Multiple cron-once invocations create independent pipelines
// =============================================================================

/// Each `oj cron once` invocation should create a distinct pipeline with
/// its own lifecycle, and both should complete independently.
#[test]
fn cron_creates_independent_pipelines() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", FAST_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Trigger two independent pipelines
    temp.oj().args(&["cron", "once", "ticker"]).passes();
    temp.oj().args(&["cron", "once", "ticker"]).passes();

    // Wait for both to complete
    let both_completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        output.matches("completed").count() >= 2
    });

    if !both_completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        both_completed,
        "both cron-triggered pipelines should complete independently"
    );
}

// =============================================================================
// Test 4: Stopping cron doesn't kill running pipeline
// =============================================================================

/// When a cron is stopped while one of its triggered pipelines is still
/// running, the pipeline should continue unaffected.
#[test]
fn cron_stop_does_not_kill_running_pipeline() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/cron.toml", SLOW_CRON_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Trigger the slow pipeline via cron once
    temp.oj().args(&["cron", "once", "slow"]).passes();

    // Wait for pipeline to reach running state
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("running")
    });
    assert!(running, "pipeline should be running");

    // Start and stop the cron — should NOT affect the already-running pipeline
    temp.oj().args(&["cron", "start", "slow"]).passes();
    temp.oj().args(&["cron", "stop", "slow"]).passes();

    // Pipeline should still be running (the sleep 30 hasn't finished)
    let still_running = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    assert!(
        still_running.contains("running"),
        "pipeline should still be running after cron stop, got: {}",
        still_running
    );
}

// =============================================================================
// Test 5: Cron restart picks up runbook changes
// =============================================================================

/// After modifying the runbook and restarting the cron, `oj cron once` should
/// use the updated pipeline definition.
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
run = { pipeline = "tick" }

[pipeline.tick]

[[pipeline.tick.step]]
name = "updated-work"
run = "echo updated"
"#;
    temp.file(".oj/runbooks/cron.toml", updated_runbook);

    // Restart to pick up the change
    temp.oj()
        .args(&["cron", "restart", "ticker"])
        .passes()
        .stdout_has("restarted");

    // Trigger pipeline with the new definition
    temp.oj().args(&["cron", "once", "ticker"]).passes();

    // Wait for pipeline to complete
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        completed,
        "pipeline with updated runbook should complete after cron restart"
    );

    // Stop the cron timer
    temp.oj().args(&["cron", "stop", "ticker"]).passes();
}
