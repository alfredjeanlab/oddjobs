//! Multi-job concurrency specs
//!
//! Verify that workers enforce concurrency limits, free slots correctly,
//! and operate independently across separate worker pools.

use crate::prelude::*;

/// Runbook with concurrency=2 for testing limit enforcement.
const CONCURRENCY_2_RUNBOOK: &str = r#"
[queue.tasks]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = { queue = "tasks" }
handler = { job = "process" }
concurrency = 2

[job.process]
vars = ["cmd"]

[[job.process.step]]
name = "work"
run = "${var.cmd}"
"#;

/// Runbook with two independent queue/worker/job sets.
const TWO_WORKERS_RUNBOOK: &str = r#"
[queue.alpha]
type = "persisted"
vars = ["cmd"]

[queue.beta]
type = "persisted"
vars = ["cmd"]

[worker.alpha_runner]
source = { queue = "alpha" }
handler = { job = "alpha_job" }
concurrency = 1

[worker.beta_runner]
source = { queue = "beta" }
handler = { job = "beta_job" }
concurrency = 1

[job.alpha_job]
vars = ["cmd"]

[[job.alpha_job.step]]
name = "work"
run = "${var.cmd}"

[job.beta_job]
vars = ["cmd"]

[[job.beta_job.step]]
name = "work"
run = "${var.cmd}"
"#;

// =============================================================================
// Test 1: Concurrency limit prevents dispatching all items at once
// =============================================================================

#[test]
fn concurrency_limit_caps_active_jobs() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", CONCURRENCY_2_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 4 items with blocking commands so jobs stay running.
    // Each item needs unique data to avoid deduplication.
    for i in 0..4 {
        temp.oj()
            .args(&[
                "queue",
                "push",
                "tasks",
                &format!(r#"{{"cmd": "sleep 30 && echo item-{}"}}"#, i),
            ])
            .passes();
    }

    // Wait until we observe both running jobs AND pending queue items.
    // The engine processes events asynchronously after the CLI returns, so
    // we must wait for all 4 queue pushes to be applied AND for the worker
    // to have dispatched as many items as the concurrency limit allows.
    let cap_observed = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let jobs = temp.oj().args(&["job", "list"]).passes().stdout();
        let items = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        jobs.contains("running") && items.contains("pending")
    });

    if !cap_observed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        cap_observed,
        "with concurrency=2 and blocking commands, should see running jobs \
         alongside pending queue items"
    );

    // The concurrency cap should limit running jobs to at most 2
    let job_output = temp.oj().args(&["job", "list"]).passes().stdout();
    let running_count = job_output.matches("running").count();
    assert!(
        running_count <= 2,
        "concurrency=2 should cap running jobs at 2, got {}:\n{}",
        running_count,
        job_output
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
                "tasks",
                &format!(r#"{{"cmd": "echo item-{}"}}"#, i),
            ])
            .passes();
    }

    // Wait for all 4 to complete
    let all_done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
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
// Test 3: Failed job frees slot for next pending item
// =============================================================================

/// Runbook with concurrency=1 for serial dispatch testing.
const CONCURRENCY_1_RUNBOOK: &str = r#"
[queue.tasks]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = { queue = "tasks" }
handler = { job = "process" }
concurrency = 1

[job.process]
vars = ["cmd"]

[[job.process.step]]
name = "work"
run = "${var.cmd}"
"#;

#[test]
#[serial_test::serial]
fn failed_job_frees_slot_for_next_pending_item() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", CONCURRENCY_1_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 2 items: first fails, second succeeds.
    // With concurrency=1, only one runs at a time.
    // The second item can only start if the first item's failure frees the slot.
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "exit 1"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "echo second"}"#])
        .passes();

    // Wait for the second item to complete, proving the failed job
    // freed the concurrency slot. Allow generous time for the engine to
    // process all events under parallel test load.
    let second_completed = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.contains("completed")
    });

    if !second_completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        second_completed,
        "second item should complete, proving the failed job freed the slot"
    );

    // Verify: 1 dead (the failed one), 1 completed
    let items = temp
        .oj()
        .args(&["queue", "show", "tasks"])
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
// Test 4: Worker list reflects active job count
// =============================================================================

#[test]
fn worker_list_reflects_active_job_count() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", CONCURRENCY_2_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 2 blocking items so both slots fill.
    // Each item needs unique data to avoid deduplication.
    temp.oj()
        .args(&[
            "queue",
            "push",
            "tasks",
            r#"{"cmd": "sleep 30 && echo item-0"}"#,
        ])
        .passes();
    temp.oj()
        .args(&[
            "queue",
            "push",
            "tasks",
            r#"{"cmd": "sleep 30 && echo item-1"}"#,
        ])
        .passes();

    // Wait for 2 jobs to be running
    let two_running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.matches("running").count() >= 2
    });

    if !two_running {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(two_running, "should have 2 jobs running");

    // Verify `oj worker list` shows active=2 for the runner worker
    let worker_output = temp.oj().args(&["worker", "list"]).passes().stdout();
    assert!(
        worker_output.contains("2"),
        "worker list should show 2 active jobs, got:\n{}",
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

    // Push a blocking item to each queue.
    // Note: Different queues don't share deduplication, but using unique
    // data anyway for consistency.
    temp.oj()
        .args(&[
            "queue",
            "push",
            "alpha",
            r#"{"cmd": "sleep 30 && echo alpha"}"#,
        ])
        .passes();
    temp.oj()
        .args(&[
            "queue",
            "push",
            "beta",
            r#"{"cmd": "sleep 30 && echo beta"}"#,
        ])
        .passes();

    // Wait for both jobs to be running simultaneously.
    // Each worker has concurrency=1, but since they are independent,
    // both should have 1 active job at the same time (total 2 running).
    let both_running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.matches("running").count() >= 2
    });

    if !both_running {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        both_running,
        "both workers should have 1 running job each (2 total), \
         proving independent concurrency pools"
    );

    // Verify we see both job types in the list
    let job_output = temp.oj().args(&["job", "list"]).passes().stdout();
    assert!(
        job_output.contains("alpha_job"),
        "should see alpha_job job:\n{}",
        job_output
    );
    assert!(
        job_output.contains("beta_job"),
        "should see beta_job job:\n{}",
        job_output
    );
}
