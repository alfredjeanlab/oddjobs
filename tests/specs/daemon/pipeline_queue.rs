//! Pipeline↔Queue lifecycle specs
//!
//! Verify that queue items transition correctly when their associated
//! pipeline is cancelled, fails, or completes.

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

/// Same as QUEUE_PIPELINE_RUNBOOK but concurrency = 3.
const QUEUE_PIPELINE_CONCURRENT_RUNBOOK: &str = r#"
[queue.jobs]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = { queue = "jobs" }
handler = { pipeline = "process" }
concurrency = 3

[pipeline.process]
vars = ["cmd"]

[[pipeline.process.step]]
name = "work"
run = "${var.cmd}"
"#;

/// Extract the first pipeline ID from `oj pipeline list` output
/// by matching a line containing `name_filter`.
fn extract_pipeline_id(temp: &Project, name_filter: &str) -> String {
    let output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    output
        .lines()
        .find(|l| l.contains(name_filter))
        .unwrap_or_else(|| {
            panic!(
                "no pipeline matching '{}' in output:\n{}",
                name_filter, output
            )
        })
        .split_whitespace()
        .next()
        .expect("should have an ID column")
        .to_string()
}

// =============================================================================
// Test 1: Cancel transitions queue item from active
// =============================================================================

#[test]
fn cancel_pipeline_transitions_queue_item_from_active() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a blocking command so the pipeline stays on "work" step
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "sleep 30"}"#])
        .passes();

    // Wait for the pipeline to reach "running" on the "work" step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("work") && out.contains("running")
    });
    assert!(running, "pipeline should be running the work step");

    // Verify queue item is active
    let active = temp
        .oj()
        .args(&["queue", "items", "jobs"])
        .passes()
        .stdout();
    assert!(active.contains("active"), "queue item should be active");

    // Get pipeline ID and cancel it
    let pipeline_id = extract_pipeline_id(&temp, "process");
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // Wait for queue item to leave "active" status
    let transitioned = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        !out.contains("active")
    });

    if !transitioned {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        transitioned,
        "queue item must not stay active after pipeline cancel"
    );

    // Item should be dead or failed (not stuck on active)
    let final_status = temp
        .oj()
        .args(&["queue", "items", "jobs"])
        .passes()
        .stdout();
    assert!(
        final_status.contains("dead") || final_status.contains("failed"),
        "cancelled pipeline should mark queue item as dead or failed, got: {}",
        final_status
    );
}

// =============================================================================
// Test 2: Failed pipeline marks queue item dead
// =============================================================================

#[test]
fn failed_pipeline_marks_queue_item_dead() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a command that will fail (exit 1)
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "exit 1"}"#])
        .passes();

    // Wait for queue item to reach dead status (no retry config → immediate dead)
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
    assert!(dead, "failed pipeline should mark queue item as dead");
}

// =============================================================================
// Test 3: Completed pipeline marks queue item completed + frees concurrency
// =============================================================================

#[test]
fn completed_pipeline_marks_queue_item_completed() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a command that succeeds
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo hello"}"#])
        .passes();

    // Wait for queue item to reach completed status
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
    assert!(
        completed,
        "successful pipeline should mark queue item as completed"
    );

    // Verify concurrency slot is freed by pushing another item
    // (worker concurrency = 1, so a second item can only run if the slot was freed)
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo second"}"#])
        .passes();

    let second_completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
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
fn one_pipeline_failure_does_not_affect_others() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_CONCURRENT_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 3 items: one fast-fail, two that succeed
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "exit 1"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo ok"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo ok"}"#])
        .passes();

    // Wait for all 3 items to reach terminal status
    let all_terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        !out.contains("active") && !out.contains("pending")
    });

    if !all_terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(all_terminal, "all queue items should reach terminal status");

    // Verify: 1 dead (the failed one), 2 completed
    let items_output = temp
        .oj()
        .args(&["queue", "items", "jobs"])
        .passes()
        .stdout();
    assert_eq!(
        items_output.matches("completed").count(),
        2,
        "two items should be completed, got: {}",
        items_output
    );
    assert!(
        items_output.contains("dead") || items_output.contains("failed"),
        "the failing item should be dead or failed, got: {}",
        items_output
    );
}

// =============================================================================
// Test 5: Circuit breaker — escalation after max action attempts
// =============================================================================

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
/// After exhausting attempts, the pipeline should escalate.
fn circuit_breaker_runbook(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[queue.jobs]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = {{ queue = "jobs" }}
handler = {{ pipeline = "process" }}
concurrency = 1

[pipeline.process]
vars = ["cmd"]

[[pipeline.process.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "This will fail."
on_dead = {{ action = "recover", attempts = 2 }}
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
        .args(&["queue", "push", "jobs", r#"{"cmd": "noop"}"#])
        .passes();

    // Wait for pipeline to reach "waiting" (escalated) status
    let escalated = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("waiting")
    });

    if !escalated {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        escalated,
        "pipeline should escalate to waiting after exhausting recover attempts"
    );

    // Queue item should still be active (pipeline hasn't terminated, it's waiting)
    let items = temp
        .oj()
        .args(&["queue", "items", "jobs"])
        .passes()
        .stdout();
    assert!(
        items.contains("active"),
        "queue item should remain active while pipeline is waiting for intervention, got: {}",
        items
    );
}
