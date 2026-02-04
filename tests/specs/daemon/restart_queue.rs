//! Daemon restart queue item consistency specs
//!
//! Verify that queue items maintain correct status across daemon restarts
//! (both graceful and crash recovery).

use crate::prelude::*;

/// Runbook: persisted queue + worker + shell-only pipeline.
/// Pipeline steps: work → done.
/// `work` runs a command provided via the queue item's `cmd` var.
/// `done` always succeeds (echo done).
const QUEUE_PIPELINE_RUNBOOK: &str = r#"
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

/// Queue-only runbook for testing WAL persistence without worker interference.
const QUEUE_ONLY_RUNBOOK: &str = r#"
[queue.jobs]
type = "persisted"
vars = ["cmd"]
"#;

/// Scenario for a slow agent that sleeps for a while.
/// The sleep gives us time to kill the daemon mid-pipeline.
const SLOW_AGENT_SCENARIO: &str = r#"
name = "slow-agent"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Running a slow task..."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "sleep 10" }

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#;

/// Queue-driven agent pipeline for crash recovery testing.
/// Worker takes queue items and runs an agent that sleeps.
/// on_dead = "done" advances the pipeline when the agent exits after crash.
fn crash_recovery_queue_runbook(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[queue.jobs]
type = "persisted"
vars = ["name"]

[worker.runner]
source = {{ queue = "jobs" }}
handler = {{ pipeline = "process" }}
concurrency = 1

[pipeline.process]
vars = ["name"]

[[pipeline.process.step]]
name = "work"
run = {{ agent = "slow" }}

[agent.slow]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

// =============================================================================
// Test 1: Completed items persist across restart
// =============================================================================

#[test]
fn completed_queue_items_persist_across_restart() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item that completes quickly
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo hello"}"#])
        .passes();

    // Wait for completion
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(completed, "queue item should complete before restart");

    // IPC round-trip acts as sync point for WAL group commit flush
    temp.oj().args(&["daemon", "status"]).passes();

    // Graceful restart
    temp.oj().args(&["daemon", "stop"]).passes();
    temp.oj().args(&["daemon", "start"]).passes();

    // Verify completed status persisted after WAL replay
    let still_completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.contains("completed")
    });
    assert!(
        still_completed,
        "completed status should persist across restart"
    );
}

// =============================================================================
// Test 2: Dead items persist across restart
// =============================================================================

#[test]
fn dead_queue_items_persist_across_restart() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item that fails (no retry config → immediate dead)
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "exit 1"}"#])
        .passes();

    // Wait for dead status
    let dead = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.contains("dead")
    });

    if !dead {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(dead, "queue item should be dead before restart");

    // IPC round-trip acts as sync point for WAL group commit flush
    temp.oj().args(&["daemon", "status"]).passes();

    // Graceful restart
    temp.oj().args(&["daemon", "stop"]).passes();
    temp.oj().args(&["daemon", "start"]).passes();

    // Verify dead status persisted after WAL replay
    let still_dead = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.contains("dead")
    });
    assert!(still_dead, "dead status should persist across restart");
}

// =============================================================================
// Test 3: Pending items persist across restart (no worker)
// =============================================================================

#[test]
fn pending_queue_items_persist_across_restart() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_ONLY_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Push two items (no worker defined, so they stay pending)
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo hello"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo world"}"#])
        .passes();

    // Wait for both items to appear as pending (WAL commit is async)
    let both_pending = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.matches("pending").count() == 2
    });
    assert!(both_pending, "both items should be pending");

    // Graceful restart
    temp.oj().args(&["daemon", "stop"]).passes();
    temp.oj().args(&["daemon", "start"]).passes();

    // Verify items survived restart with correct status and data
    let out = temp
        .oj()
        .args(&["queue", "items", "jobs"])
        .passes()
        .stdout();
    assert_eq!(
        out.matches("pending").count(),
        2,
        "both items should still be pending after restart, got: {}",
        out
    );
    assert!(
        out.contains("echo hello"),
        "first item data should survive restart, got: {}",
        out
    );
    assert!(
        out.contains("echo world"),
        "second item data should survive restart, got: {}",
        out
    );
}

// =============================================================================
// Test 4: Worker resumes and processes new items after restart
// =============================================================================

#[test]
fn worker_resumes_and_processes_new_items_after_restart() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Complete one item to verify worker is functional
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo first"}"#])
        .passes();

    let first_done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.contains("completed")
    });

    if !first_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(first_done, "first item should complete before restart");

    // Graceful restart
    temp.oj().args(&["daemon", "stop"]).passes();
    temp.oj().args(&["daemon", "start"]).passes();

    // Push new item after restart — recovered worker should process it
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo second"}"#])
        .passes();

    let second_done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.matches("completed").count() >= 2
    });

    if !second_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        second_done,
        "worker should process new items after daemon restart"
    );
}

// =============================================================================
// Test 5: Active queue item completes after daemon crash recovery
// =============================================================================

/// When the daemon crashes while a queue item's pipeline is running an agent,
/// restarting the daemon triggers reconciliation which detects the dead agent,
/// fires on_dead = "done" to advance the pipeline, and the worker marks the
/// queue item as completed.
#[test]
fn active_queue_item_completes_after_daemon_crash() {
    let temp = Project::empty();
    temp.git_init();

    // Set up scenario and runbook
    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/queue.toml",
        &crash_recovery_queue_runbook(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item — the agent will start running a slow task
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"name": "crash-test"}"#])
        .passes();

    // Wait for the queue item to become active and the pipeline to reach running
    let active = wait_for(SPEC_WAIT_MAX_MS, || {
        let items = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        let pipelines = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        items.contains("active") && pipelines.contains("running")
    });
    assert!(
        active,
        "queue item should be active with a running pipeline"
    );

    // Kill the daemon with SIGKILL (simulates crash)
    let killed = temp.daemon_kill();
    assert!(killed, "should be able to kill daemon");

    // Wait for daemon to actually die.
    // Use raw command output because the daemon may return connection errors
    // (exit code 1) during the transient death window.
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

    // Restart the daemon — triggers reconciliation
    temp.oj().args(&["daemon", "start"]).passes();

    // Wait for the pipeline to complete via recovery (on_dead = "done")
    // and the queue item to reach completed status
    let item_completed = wait_for(SPEC_WAIT_MAX_MS * 10, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        out.contains("completed")
    });

    if !item_completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        eprintln!(
            "=== QUEUE ITEMS ===\n{}\n=== END ITEMS ===",
            temp.oj()
                .args(&["queue", "items", "jobs"])
                .passes()
                .stdout()
        );
        eprintln!(
            "=== PIPELINES ===\n{}\n=== END PIPELINES ===",
            temp.oj().args(&["pipeline", "list"]).passes().stdout()
        );
    }
    assert!(
        item_completed,
        "queue item should complete after daemon crash recovery"
    );
}
