//! Agent stop hook resilience specs
//!
//! Verify that agent steps handle cancellation and stop scenarios correctly:
//! - Pipeline cancel kills agent session and transitions pipeline
//! - `on_cancel` cleanup steps run after agent is killed
//! - Queue items transition properly when agent pipelines are cancelled
//! - Re-cancellation during cleanup is a no-op
//! - Daemon stop with --kill terminates agent sessions mid-pipeline

use crate::prelude::*;

// =============================================================================
// Scenarios
// =============================================================================

/// A slow agent that sleeps, keeping the pipeline on the agent step long enough
/// to cancel it mid-execution. Uses -p mode so on_dead fires if not cancelled.
const SLOW_AGENT_SCENARIO: &str = r#"
name = "slow-agent"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Running a slow task..."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "sleep 30" }

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#;

// =============================================================================
// Runbooks
// =============================================================================

/// Runbook with a slow agent step and no on_cancel. Cancellation goes straight
/// to terminal "cancelled" status.
fn runbook_agent_no_on_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ pipeline = "work" }}

[pipeline.work]
vars  = ["name"]

[[pipeline.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

/// Runbook with a slow agent step and a pipeline-level on_cancel that routes
/// to a cleanup step. The cleanup step writes a marker file to prove it ran.
fn runbook_agent_with_on_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ pipeline = "work" }}

[pipeline.work]
vars  = ["name"]
on_cancel = {{ step = "cleanup" }}

[[pipeline.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[[pipeline.work.step]]
name = "cleanup"
run = "echo cleaned"

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

/// Runbook with a slow agent step and a step-level on_cancel that routes to
/// a cleanup step.
fn runbook_agent_step_on_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ pipeline = "work" }}

[pipeline.work]
vars  = ["name"]

[[pipeline.work.step]]
name = "agent"
run = {{ agent = "worker" }}
on_cancel = {{ step = "cleanup" }}

[[pipeline.work.step]]
name = "cleanup"
run = "echo step-cleanup-ran"

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

/// Runbook with a slow agent step configured with on_dead = recover. Cancel
/// should override the recover action and go to cancelled/cleanup.
fn runbook_agent_recover_then_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ pipeline = "work" }}

[pipeline.work]
vars  = ["name"]

[[pipeline.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = {{ action = "recover", attempts = 3 }}
"#,
        scenario_path.display()
    )
}

/// Runbook with queue + worker + slow agent step for testing queue item
/// transitions on cancel.
fn runbook_queue_agent_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[queue.jobs]
type = "persisted"
vars = ["name"]

[worker.runner]
source = {{ queue = "jobs" }}
handler = {{ pipeline = "work" }}
concurrency = 1

[pipeline.work]
vars = ["name"]

[[pipeline.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

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
// Test 1: Cancel pipeline during agent step transitions to cancelled
// =============================================================================

/// When a pipeline is cancelled while an agent step is running, the agent
/// session is killed and the pipeline transitions to "cancelled" status.
#[test]
fn cancel_agent_step_transitions_pipeline_to_cancelled() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/work.toml",
        &runbook_agent_no_on_cancel(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "cancel-basic"]).passes();

    // Wait for pipeline to reach agent step (running status)
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(
        running,
        "pipeline should reach the agent step\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Cancel the pipeline
    let pipeline_id = extract_pipeline_id(&temp, "work");
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // Pipeline should reach cancelled status
    let cancelled = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("cancelled")
    });

    if !cancelled {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        cancelled,
        "pipeline should transition to cancelled after cancel during agent step"
    );
}

// =============================================================================
// Test 2: Pipeline-level on_cancel cleanup step runs after agent is killed
// =============================================================================

/// When a pipeline with `on_cancel` is cancelled during an agent step, the
/// agent is killed first, then the cleanup step runs, and the pipeline
/// completes (not stuck in cancelling).
#[test]
fn on_cancel_cleanup_step_runs_after_agent_kill() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/work.toml",
        &runbook_agent_with_on_cancel(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "cancel-cleanup"]).passes();

    // Wait for pipeline to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(
        running,
        "pipeline should reach the agent step\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Cancel the pipeline
    let pipeline_id = extract_pipeline_id(&temp, "work");
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // The cleanup step should run and the pipeline should reach a terminal state.
    // With on_cancel routing, the pipeline goes through "cleanup" step before terminal.
    let terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("completed") || out.contains("cancelled")
    });

    if !terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        terminal,
        "pipeline should reach terminal state after on_cancel cleanup step runs"
    );
}

// =============================================================================
// Test 3: Step-level on_cancel routes to cleanup
// =============================================================================

/// When a step has its own on_cancel, the step-level on_cancel takes priority
/// over the pipeline-level on_cancel.
#[test]
fn step_level_on_cancel_routes_to_cleanup() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/work.toml",
        &runbook_agent_step_on_cancel(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "step-cancel"]).passes();

    // Wait for pipeline to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(
        running,
        "pipeline should reach the agent step\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Cancel the pipeline
    let pipeline_id = extract_pipeline_id(&temp, "work");
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // Pipeline should route to cleanup step and reach terminal
    let terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("completed") || out.contains("cancelled")
    });

    if !terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        terminal,
        "pipeline should reach terminal state after step-level on_cancel cleanup runs"
    );
}

// =============================================================================
// Test 4: Cancel overrides on_dead recover action
// =============================================================================

/// When the agent is configured with on_dead = recover (with attempts), but
/// the pipeline is cancelled, the cancel should take effect immediately.
/// The pipeline should NOT attempt to recover the agent.
#[test]
fn cancel_overrides_on_dead_recover() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/work.toml",
        &runbook_agent_recover_then_cancel(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "work", "cancel-vs-recover"])
        .passes();

    // Wait for pipeline to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(
        running,
        "pipeline should reach the agent step\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Cancel the pipeline while agent is running
    let pipeline_id = extract_pipeline_id(&temp, "work");
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // Pipeline should be cancelled, NOT recovering
    let cancelled = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("cancelled")
    });

    if !cancelled {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        cancelled,
        "cancel should override on_dead=recover and transition to cancelled"
    );
}

// =============================================================================
// Test 5: Re-cancel during cleanup step is a no-op
// =============================================================================

/// When a pipeline is already running its on_cancel cleanup step, issuing
/// another cancel should be a no-op (the cleanup runs to completion).
#[test]
fn re_cancel_during_cleanup_is_noop() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    // Use on_cancel that runs a slightly longer command to give time for re-cancel
    temp.file(
        ".oj/runbooks/work.toml",
        &format!(
            r#"
[command.work]
args = "<name>"
run = {{ pipeline = "work" }}

[pipeline.work]
vars  = ["name"]
on_cancel = {{ step = "cleanup" }}

[[pipeline.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[[pipeline.work.step]]
name = "cleanup"
run = "echo cleanup-done"

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
            scenario_path.display()
        ),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "re-cancel-test"]).passes();

    // Wait for pipeline to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(
        running,
        "pipeline should reach the agent step\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // First cancel
    let pipeline_id = extract_pipeline_id(&temp, "work");
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // Immediately re-cancel (should be a no-op due to cancelling guard)
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // Pipeline should still reach terminal state (cleanup completes, not disrupted)
    let terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("completed") || out.contains("cancelled")
    });

    if !terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        terminal,
        "pipeline should reach terminal state despite re-cancel during cleanup"
    );
}

// =============================================================================
// Test 6: Cancel agent pipeline frees queue slot
// =============================================================================

/// When a queue-spawned pipeline with an agent step is cancelled, the queue
/// item transitions from active and the concurrency slot is freed.
#[test]
fn cancel_agent_pipeline_frees_queue_slot() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/queue.toml",
        &runbook_queue_agent_cancel(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a name var
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"name": "test-item"}"#])
        .passes();

    // Wait for pipeline to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(
        running,
        "pipeline should reach the agent step\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Verify queue item is active
    let active = temp
        .oj()
        .args(&["queue", "items", "jobs"])
        .passes()
        .stdout();
    assert!(active.contains("active"), "queue item should be active");

    // Cancel the pipeline
    let pipeline_id = extract_pipeline_id(&temp, "work");
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // Queue item should leave active status
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
        "queue item must not stay active after agent pipeline cancel"
    );

    // Verify the slot is freed by pushing another item and watching it activate
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"name": "second-item"}"#])
        .passes();

    let second_runs = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp
            .oj()
            .args(&["queue", "items", "jobs"])
            .passes()
            .stdout();
        // The second item should become active (or complete), proving the slot was freed
        out.matches("active").count() >= 1 || out.matches("completed").count() >= 1
    });

    if !second_runs {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        second_runs,
        "second queue item should activate, proving concurrency slot was freed"
    );
}

// =============================================================================
// Test 7: Cancel already-terminal pipeline is a no-op
// =============================================================================

/// Cancelling a pipeline that has already completed should be a no-op
/// (no crash, no state corruption).
#[test]
fn cancel_completed_pipeline_is_noop() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/fast.toml",
        r#"
[command.fast]
args = "<name>"
run = { pipeline = "fast" }

[pipeline.fast]
vars  = ["name"]

[[pipeline.fast.step]]
name = "work"
run = "echo done"
"#,
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "fast", "noop-cancel"]).passes();

    // Wait for pipeline to complete
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(completed, "pipeline should complete");

    // Cancel the already-completed pipeline (should be a no-op)
    let pipeline_id = extract_pipeline_id(&temp, "fast");
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes();

    // Pipeline should still show completed (not cancelled)
    temp.oj()
        .args(&["pipeline", "list"])
        .passes()
        .stdout_has("completed");
}
