//! Job↔Queue lifecycle specs
//!
//! Verify that queue items transition correctly when their associated
//! job is cancelled, fails, or completes.

use crate::prelude::*;

/// Runbook: persisted queue + worker + shell-only job.
/// Job steps: work → done.
/// `work` runs a command provided via the queue item's `cmd` var.
/// `done` always succeeds (echo done).
const QUEUE_JOB_RUNBOOK: &str = r#"
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

/// Same as QUEUE_JOB_RUNBOOK but concurrency = 3.
const QUEUE_JOB_CONCURRENT_RUNBOOK: &str = r#"
[queue.tasks]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = { queue = "tasks" }
handler = { job = "process" }
concurrency = 3

[job.process]
vars = ["cmd"]

[[job.process.step]]
name = "work"
run = "${var.cmd}"
"#;

/// Extract the first job ID from `oj job list` output
/// by matching a line containing `name_filter`.
fn extract_job_id(temp: &Project, name_filter: &str) -> String {
    let output = temp.oj().args(&["job", "list"]).passes().stdout();
    output
        .lines()
        .find(|l| l.contains(name_filter))
        .unwrap_or_else(|| panic!("no job matching '{}' in output:\n{}", name_filter, output))
        .split_whitespace()
        .next()
        .expect("should have an ID column")
        .to_string()
}

// =============================================================================
// Test 1: Cancel transitions queue item from active
// =============================================================================

#[test]
fn cancel_job_transitions_queue_item_from_active() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_JOB_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a blocking command so the job stays on "work" step
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "sleep 30"}"#])
        .passes();

    // Wait for the job to reach "running" on the "work" step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("work") && out.contains("running")
    });
    assert!(running, "job should be running the work step");

    // Verify queue item is active
    let active = temp
        .oj()
        .args(&["queue", "show", "tasks"])
        .passes()
        .stdout();
    assert!(active.contains("active"), "queue item should be active");

    // Get job ID and cancel it
    let job_id = extract_job_id(&temp, "process");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Wait for queue item to reach a terminal status (dead or failed)
    let transitioned = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.contains("dead") || out.contains("failed")
    });

    if !transitioned {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        transitioned,
        "cancelled job should mark queue item as dead or failed"
    );
}

// =============================================================================
// Test 2: Failed job marks queue item dead
// =============================================================================

#[test]
fn failed_job_marks_queue_item_dead() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_JOB_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a command that will fail (exit 1)
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "exit 1"}"#])
        .passes();

    // Wait for queue item to reach dead status (no retry config → immediate dead)
    let dead = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.contains("dead")
    });

    if !dead {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(dead, "failed job should mark queue item as dead");
}

// =============================================================================
// Test 3: Completed job marks queue item completed + frees concurrency
// =============================================================================

#[test]
fn completed_job_marks_queue_item_completed() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_JOB_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a command that succeeds
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "echo hello"}"#])
        .passes();

    // Wait for queue item to reach completed status
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        completed,
        "successful job should mark queue item as completed"
    );

    // Verify concurrency slot is freed by pushing another item
    // (worker concurrency = 1, so a second item can only run if the slot was freed)
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "echo second"}"#])
        .passes();

    let second_completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        // Both items should be completed
        out.matches("completed").count() >= 2
    });

    if !second_completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        second_completed,
        "second item should complete, proving concurrency slot was freed"
    );
}

// =============================================================================
// Test 4: Multi-item isolation — one failure doesn't affect others
// =============================================================================

#[test]
fn one_job_failure_does_not_affect_others() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_JOB_CONCURRENT_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 3 items: one fast-fail, two that succeed.
    // Each item needs unique data to avoid deduplication.
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "exit 1"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "echo ok-1"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "echo ok-2"}"#])
        .passes();

    // Wait for all 3 items to reach expected terminal status:
    // 2 completed (the successful ones) + 1 dead/failed (the `exit 1`).
    // Capture the output from within the wait_for to avoid TOCTOU races
    // from a separate query seeing a different snapshot.
    let mut items_output = String::new();
    let all_terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        items_output = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        let completed = items_output.matches("completed").count();
        let dead_or_failed =
            items_output.matches("dead").count() + items_output.matches("failed").count();
        completed >= 2 && dead_or_failed >= 1
    });

    if !all_terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        all_terminal,
        "should have 2 completed and 1 dead/failed, got: {}",
        items_output
    );
}

// =============================================================================
// Test 5: Circuit breaker — escalation after max action attempts
// =============================================================================

// =============================================================================
// Test 6: Queue item released after daemon crash with terminal job
// =============================================================================

/// When the daemon crashes after a job reaches terminal state but before
/// the QueueCompleted event is persisted to the WAL, restarting the daemon
/// should reconcile the queue item during worker recovery.
///
/// Uses a fast shell command so the job completes quickly, then kills
/// the daemon and verifies the queue item is completed after restart.
#[test]
fn queue_item_released_after_crash_with_terminal_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_JOB_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a fast command so the job completes quickly
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "echo hello"}"#])
        .passes();

    // Wait for the job to reach a terminal state
    let job_done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("completed")
    });
    assert!(job_done, "job should complete before crash");

    // Kill the daemon (simulates crash — queue event may or may not be persisted)
    let killed = temp.daemon_kill();
    assert!(killed, "should be able to kill daemon");

    // Wait for daemon to actually die
    let daemon_dead = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp
            .oj()
            .args(&["daemon", "status"])
            .command()
            .output()
            .expect("command should run");
        let stdout = String::from_utf8_lossy(&output.stdout);
        !stdout.contains("Status: running")
    });
    assert!(daemon_dead, "daemon should be dead after kill");

    // Restart the daemon — triggers worker recovery + reconciliation
    temp.oj().args(&["daemon", "start"]).passes();

    // Queue item should be completed (either from original WAL or reconciliation)
    let item_completed = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.contains("completed")
    });

    if !item_completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        eprintln!(
            "=== QUEUE ITEMS ===\n{}\n=== END ITEMS ===",
            temp.oj()
                .args(&["queue", "show", "tasks"])
                .passes()
                .stdout()
        );
        eprintln!(
            "=== JOBS ===\n{}\n=== END JOBS ===",
            temp.oj().args(&["job", "list"]).passes().stdout()
        );
    }
    assert!(
        item_completed,
        "queue item should be completed after daemon crash recovery with terminal job"
    );
}

/// Scenario that makes the agent exit immediately (print mode) with a response.
const FAILING_AGENT_SCENARIO: &str = r#"
name = "failing-agent"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "I cannot complete this task."

[tool_execution]
mode = "live"
"#;

/// Runbook where the agent always exits (via -p mode),
/// and on_dead is configured with limited recover attempts.
/// After exhausting attempts, the job should escalate.
fn circuit_breaker_runbook(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[queue.tasks]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = {{ queue = "tasks" }}
handler = {{ job = "process" }}
concurrency = 1

[job.process]
vars = ["cmd"]

[[job.process.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "This will fail."
on_dead = {{ action = "resume", attempts = 2 }}
"#,
        scenario_path.display()
    )
}

#[test]
fn circuit_breaker_escalates_after_max_attempts() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/fail.toml", FAILING_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/fail.toml");
    temp.file(
        ".oj/runbooks/queue.toml",
        &circuit_breaker_runbook(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push an item — the agent will exit, recover, exit, recover, then escalate
    temp.oj()
        .args(&["queue", "push", "tasks", r#"{"cmd": "noop"}"#])
        .passes();

    // Wait for job to reach "waiting" (escalated) status
    let escalated = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("waiting")
    });

    if !escalated {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        escalated,
        "job should escalate to waiting after exhausting recover attempts"
    );

    // Queue item should still be active (job hasn't terminated, it's waiting)
    let items = temp
        .oj()
        .args(&["queue", "show", "tasks"])
        .passes()
        .stdout();
    assert!(
        items.contains("active"),
        "queue item should remain active while job is waiting for intervention, got: {}",
        items
    );
}
