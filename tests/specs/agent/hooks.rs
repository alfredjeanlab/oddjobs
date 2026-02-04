//! Agent hook tests for socket propagation.
//!
//! Verifies that hooks running inside an agent's tmux session can communicate
//! with the daemon via the OJ_STATE_DIR socket path.

use crate::prelude::*;

// =============================================================================
// Scenarios
// =============================================================================

/// Agent emits a signal via `oj emit agent:signal` in a Bash tool call,
/// then creates a marker file to prove the emit succeeded.
fn scenario_emit_signal() -> &'static str {
    r#"
name = "emit-signal"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Signaling completion to daemon."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "oj emit agent:signal --agent $AGENT_ID '{\"action\":\"complete\"}' && touch emit-ok" }

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#
}

// =============================================================================
// Runbooks
// =============================================================================

/// Runbook that passes AGENT_ID to the agent and gates on the emit marker file.
///
/// If socket propagation works:
///   1. `oj emit agent:signal` connects to daemon via OJ_STATE_DIR → succeeds
///   2. `touch emit-ok` runs (due to &&) → file created
///   3. Gate `test -f emit-ok` passes → pipeline completes
///
/// If socket propagation is broken:
///   1. `oj emit agent:signal` fails (wrong/missing socket) → non-zero exit
///   2. `touch emit-ok` skipped (due to &&) → no file
///   3. Gate `test -f emit-ok` fails → pipeline escalates to waiting
fn runbook_stop_hook_socket(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ pipeline = "build" }}

[pipeline.build]
vars  = ["name"]

[[pipeline.build.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Signal completion."
on_dead = {{ action = "gate", run = "test -f emit-ok" }}
env = {{ AGENT_ID = "${{agent_id}}" }}
"#,
        scenario_path.display()
    )
}

// =============================================================================
// Tests
// =============================================================================

/// Tests that OJ_STATE_DIR propagates through tmux session so agent hooks
/// can reach the daemon socket.
///
/// Lifecycle: agent spawns in tmux → Bash tool runs `oj emit agent:signal`
/// (which connects to daemon via OJ_STATE_DIR) → emit succeeds → marker file
/// created → agent exits (-p mode) → on_dead gate checks marker → pipeline
/// completes.
///
/// This proves the full socket propagation chain:
///   daemon (OJ_STATE_DIR) → spawn.rs env list → tmux -e → agent env →
///   `oj emit` → daemon_socket() → daemon.sock
#[test]
fn agent_stop_hook_socket_propagation() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/emit-signal.toml", scenario_emit_signal());

    let scenario_path = temp.path().join(".oj/scenarios/emit-signal.toml");
    let runbook = runbook_stop_hook_socket(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "socket-test"]).passes();

    // Wait for pipeline to complete. If socket propagation works, the agent
    // successfully calls `oj emit` and the gate passes. If broken, the gate
    // fails and the pipeline escalates to "waiting" instead of "completed".
    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(
        done,
        "pipeline should complete via gate after successful oj emit \
         (proves OJ_STATE_DIR socket propagation)\npipeline list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["pipeline", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}
