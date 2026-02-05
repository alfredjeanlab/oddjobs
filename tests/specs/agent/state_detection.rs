//! Integration tests for agent state detection edge cases.
//!
//! Tests edge cases in the watcher's state detection logic:
//! - Rapid state transitions (working → idle → working)
//! - Incomplete JSON lines in session log
//! - Agent process death during log write

use crate::prelude::*;

// =============================================================================
// Scenarios
// =============================================================================

/// Scenario that produces multiple tool calls (rapid state transitions).
///
/// Agent does: think → tool → respond, creating multiple state transitions.
fn scenario_multi_turn() -> &'static str {
    r#"
name = "multi-turn"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Let me do several things."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "echo step1" }

[[responses]]
pattern = { type = "contains", text = "step1" }

[responses.response]
text = "First step done. Now second."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "echo step2" }

[[responses]]
pattern = { type = "contains", text = "step2" }

[responses.response]
text = "All done!"

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#
}

/// Scenario that exits immediately (print mode).
fn scenario_quick_exit() -> &'static str {
    r#"
name = "quick-exit"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Done."
"#
}

/// Scenario that produces an idle state (interactive mode, text only).
fn scenario_simple_idle() -> &'static str {
    r#"
name = "simple-idle"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "I've completed the analysis. Here's my response."
"#
}

// =============================================================================
// Runbooks
// =============================================================================

fn runbook_multi_turn(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {}"
prompt = "Do several things step by step."
on_idle = "done"
env = {{ OJ_STEP = "work" }}
"#,
        scenario_path.display()
    )
}

fn runbook_quick_exit(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Say done."
on_dead = "done"
env = {{ OJ_STEP = "work" }}
"#,
        scenario_path.display()
    )
}

// =============================================================================
// Rapid State Transition Tests
// =============================================================================

/// Tests that rapid state transitions (working → idle → working → idle) are
/// detected correctly during multi-turn agent interactions.
///
/// The agent performs multiple tool calls, each causing a state transition:
/// - prompt → working (processing)
/// - tool_use → working (tool executing)
/// - tool_result → working (processing result)
/// - text only → idle (no more tool calls)
/// - repeat for multiple turns
///
/// The watcher must detect each transition without missing intermediate states.
#[test]
fn rapid_state_transitions_detected_correctly() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_multi_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_multi_turn(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "rapid-test"]).passes();

    // With multi-turn responses, the agent goes through multiple state transitions.
    // The on_idle = done should fire after the final text-only response.
    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(
        done,
        "job should complete after multi-turn agent finishes\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

// =============================================================================
// Process Death Handling Tests
// =============================================================================

/// Tests that agent process death during log write is handled gracefully.
///
/// When an agent exits while writing to the session log, the final line may
/// be incomplete. The watcher must:
/// 1. Not crash on partial/invalid JSON
/// 2. Detect the process death via liveness check
/// 3. Emit the appropriate AgentExited/AgentGone event
///
/// Uses claudeless -p which exits immediately after one response. The rapid
/// exit creates a race between log writing and process death detection.
#[test]
fn agent_death_during_log_write_handled_gracefully() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_quick_exit());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_quick_exit(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "death-test"]).passes();

    // The agent exits immediately after one response.
    // The watcher should detect process death and fire on_dead = done.
    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(
        done,
        "job should complete after agent process death\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );

    // Verify daemon is still running (didn't crash on partial log)
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("running");
}

/// Tests multiple rapid agent spawns and deaths in sequence.
///
/// Each agent exits quickly (print mode). The daemon must handle the
/// rapid succession of spawn → log write → death cycles without crashing.
#[test]
fn multiple_rapid_agent_deaths_handled() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_quick_exit());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");

    // Create a job with multiple sequential agent steps
    let runbook = format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "step1"
run = {{ agent = "worker1" }}
on_done = "step2"

[[job.build.step]]
name = "step2"
run = {{ agent = "worker2" }}
on_done = "step3"

[[job.build.step]]
name = "step3"
run = {{ agent = "worker3" }}

[agent.worker1]
run = "claudeless --scenario {} -p"
prompt = "First."
on_dead = "done"

[agent.worker2]
run = "claudeless --scenario {} -p"
prompt = "Second."
on_dead = "done"

[agent.worker3]
run = "claudeless --scenario {} -p"
prompt = "Third."
on_dead = "done"
"#,
        scenario_path.display(),
        scenario_path.display(),
        scenario_path.display()
    );
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "multi-death"]).passes();

    // Wait for all three sequential agent steps to complete
    let done = wait_for(SPEC_WAIT_MAX_MS * 10, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(
        done,
        "job should complete after 3 sequential agents\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );

    // Daemon should still be healthy
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("running");
}

// =============================================================================
// Edge Case Tests
// =============================================================================

/// Tests that a simple idle detection works (baseline for edge cases).
///
/// This establishes a baseline for comparison with edge case tests.
#[test]
fn baseline_idle_detection_works() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_simple_idle());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {}"
prompt = "Analyze this."
on_idle = "done"
"#,
        scenario_path.display()
    );
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "baseline-test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(
        done,
        "baseline idle detection should work\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}
