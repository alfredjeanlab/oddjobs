//! Multi-pipeline concurrency specs
//!
//! Verify that workers enforce concurrency limits, free slots correctly,
//! and operate independently across separate worker pools.

use crate::prelude::*;

/// Runbook with concurrency=2 for testing limit enforcement.
const CONCURRENCY_2_RUNBOOK: &str = r#"
[queue.jobs]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = { queue = "jobs" }
handler = { pipeline = "process" }
concurrency = 2

[pipeline.process]
vars = ["cmd"]

[[pipeline.process.step]]
name = "work"
run = "${var.cmd}"
"#;

/// Runbook with two independent queue/worker/pipeline sets.
const TWO_WORKERS_RUNBOOK: &str = r#"
[queue.alpha]
type = "persisted"
vars = ["cmd"]

[queue.beta]
type = "persisted"
vars = ["cmd"]

[worker.alpha_runner]
source = { queue = "alpha" }
handler = { pipeline = "alpha_job" }
concurrency = 1

[worker.beta_runner]
source = { queue = "beta" }
handler = { pipeline = "beta_job" }
concurrency = 1

[pipeline.alpha_job]
vars = ["cmd"]

[[pipeline.alpha_job.step]]
name = "work"
run = "${var.cmd}"

[pipeline.beta_job]
vars = ["cmd"]

[[pipeline.beta_job.step]]
name = "work"
run = "${var.cmd}"
"#;

// =============================================================================
// Test 1: Concurrency limit prevents dispatching all items at once
// =============================================================================

#[test]
fn concurrency_limit_caps_active_pipelines() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", CONCURRENCY_2_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 4 items with blocking commands so pipelines stay running
    for _ in 0..4 {
        temp.oj()
            .args(&["queue", "push", "jobs", r#"{"cmd": "sleep 30"}"#])
            .passes();
    }

    // Wait until we observe both running pipelines AND pending queue items.
    // The engine processes events asynchronously after the CLI returns, so
    // we must wait for all 4 queue pushes to be applied AND for the worker
    // to have dispatched as many items as the concurrency limit allows.
    let cap_observed = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let pipelines = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        let items = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        pipelines.contains("running") && items.contains("pending")
    });

    if !cap_observed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        cap_observed,
        "with concurrency=2 and blocking commands, should see running pipelines \
         alongside pending queue items"
    );

    // The concurrency cap should limit running pipelines to at most 2
    let pipeline_output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    let running_count = pipeline_output.matches("running").count();
    assert!(
        running_count <= 2,
        "concurrency=2 should cap running pipelines at 2, got {}:\n{}",
        running_count,
        pipeline_output
    );
}

// =============================================================================
// Test 2: Pending items drain through limited concurrency slots
// =============================================================================

#[test]
fn pending_items_drain_through_limited_slots() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", CONCURRENCY_2_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 4 fast items — only 2 can run at once, so the second pair
    // must wait for the first pair to complete before starting
    for i in 1..=4 {
        temp.oj()
            .args(&[
                "queue",
                "push",
                "jobs",
                &format!(r#"{{"cmd": "echo item-{}"}}"#, i),
            ])
            .passes();
    }

    // Wait for all 4 to complete
    let all_done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.matches("completed").count() >= 4
    });

    if !all_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        all_done,
        "all 4 items should complete, proving pending items drained through slots"
    );
}

// =============================================================================
// Test 3: Failed pipeline frees slot for next pending item
// =============================================================================

/// Runbook with concurrency=1 for serial dispatch testing.
const CONCURRENCY_1_RUNBOOK: &str = r#"
[queue.jobs]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = { queue = "jobs" }
handler = { pipeline = "process" }
concurrency = 1

[pipeline.process]
vars = ["cmd"]

[[pipeline.process.step]]
name = "work"
run = "${var.cmd}"
"#;

#[test]
fn failed_pipeline_frees_slot_for_next_pending_item() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", CONCURRENCY_1_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 2 items: first fails, second succeeds.
    // With concurrency=1, only one runs at a time.
    // The second item can only start if the first item's failure frees the slot.
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "exit 1"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo second"}"#])
        .passes();

    // Wait for the second item to complete, proving the failed pipeline
    // freed the concurrency slot. Allow generous time for the engine to
    // process all events under parallel test load.
    let second_completed = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.contains("completed")
    });

    if !second_completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        second_completed,
        "second item should complete, proving the failed pipeline freed the slot"
    );

    // Verify: 1 dead (the failed one), 1 completed
    let items = temp
        .oj()
        .args(&["queue", "items", "jobs"])
        .passes()
        .stdout();
    assert!(
        items.contains("completed"),
        "second item should be completed, got:\n{}",
        items
    );
    assert!(
        items.contains("dead"),
        "the failed item should be dead, got:\n{}",
        items
    );
}

// =============================================================================
// Test 4: Worker list reflects active pipeline count
// =============================================================================

#[test]
fn worker_list_reflects_active_pipeline_count() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", CONCURRENCY_2_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 2 blocking items so both slots fill
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "sleep 30"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "sleep 30"}"#])
        .passes();

    // Wait for 2 pipelines to be running
    let two_running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.matches("running").count() >= 2
    });

    if !two_running {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(two_running, "should have 2 pipelines running");

    // Verify `oj worker list` shows active=2 for the runner worker
    let worker_output = temp.oj().args(&["worker", "list"]).passes().stdout();
    assert!(
        worker_output.contains("2"),
        "worker list should show 2 active pipelines, got:\n{}",
        worker_output
    );
}

// =============================================================================
// Test 5: Independent workers have separate concurrency pools
// =============================================================================

#[test]
fn independent_workers_have_separate_concurrency_pools() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", TWO_WORKERS_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["worker", "start", "alpha_runner"])
        .passes();
    temp.oj().args(&["worker", "start", "beta_runner"]).passes();

    // Push a blocking item to each queue
    temp.oj()
        .args(&["queue", "push", "alpha", r#"{"cmd": "sleep 30"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "beta", r#"{"cmd": "sleep 30"}"#])
        .passes();

    // Wait for both pipelines to be running simultaneously.
    // Each worker has concurrency=1, but since they are independent,
    // both should have 1 active pipeline at the same time (total 2 running).
    let both_running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.matches("running").count() >= 2
    });

    if !both_running {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        both_running,
        "both workers should have 1 running pipeline each (2 total), \
         proving independent concurrency pools"
    );

    // Verify we see both pipeline types in the list
    let pipeline_output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    assert!(
        pipeline_output.contains("alpha_job"),
        "should see alpha_job pipeline:\n{}",
        pipeline_output
    );
    assert!(
        pipeline_output.contains("beta_job"),
        "should see beta_job pipeline:\n{}",
        pipeline_output
    );
}
