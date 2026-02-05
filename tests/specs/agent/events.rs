//! Agent event handling tests using claudeless simulator.
//!
//! Tests for on_idle, on_dead, and on_error action handlers including
//! nudge, done, fail, recover, and escalate.

use crate::prelude::*;

// =============================================================================
// Scenarios
// =============================================================================

/// Agent stops at end_turn (no tool calls) - triggers on_idle
fn scenario_end_turn() -> &'static str {
    r#"
name = "end-turn"

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "I've analyzed the task and here's my response."
"#
}

/// Agent completes after being nudged
fn scenario_nudge_then_done() -> &'static str {
    r#"
name = "nudge-then-done"

[[responses]]
pattern = { type = "any" }
response = "I'm thinking about this..."
turns = [
    { expect = { type = "contains", text = "continue" }, response = "Ah, right! Let me complete this." }
]
"#
}

/// First request rate limited, then succeeds
fn scenario_rate_limit() -> &'static str {
    r#"
name = "rate-limit-recovery"

[[responses]]
pattern = { type = "any" }
failure = { type = "rate_limit", retry_after = 1 }
max_matches = 1

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Recovered from rate limit. Completing task."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "echo done" }
"#
}

/// All requests fail with network error
fn scenario_network_failure() -> &'static str {
    r#"
name = "network-failure"

[[responses]]
pattern = { type = "any" }
failure = { type = "network_unreachable" }
"#
}

// =============================================================================
// Runbooks
// =============================================================================

fn runbook_idle_done(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "execute"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {}"
prompt = "Do the task."
on_idle = "done"
"#,
        scenario_path.display()
    )
}

fn runbook_idle_nudge(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "execute"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {}"
prompt = "Do the task."
on_idle = "nudge"
"#,
        scenario_path.display()
    )
}

fn runbook_dead_done(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "execute"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Do the task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

fn runbook_dead_escalate(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "execute"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Do the task."
on_dead = "escalate"
"#,
        scenario_path.display()
    )
}

fn runbook_error_recover(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "execute"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Do the task."

[[agent.worker.on_error]]
match = "rate_limited"
action = "resume"
message = "Rate limit cleared, try again."

[[agent.worker.on_error]]
match = "no_internet"
action = "escalate"
"#,
        scenario_path.display()
    )
}

fn runbook_idle_gate_pass(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "execute"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {}"
prompt = "Do the task."
on_idle = {{ action = "gate", run = "true" }}
"#,
        scenario_path.display()
    )
}

fn runbook_idle_gate_fail(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "execute"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {}"
prompt = "Do the task."
on_idle = {{ action = "gate", run = "false" }}
"#,
        scenario_path.display()
    )
}

// =============================================================================
// on_idle tests
// =============================================================================

/// Tests that on_idle = done completes the job when agent finishes naturally
#[test]
fn on_idle_done_completes_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_idle_done(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(
        done,
        "job should complete via on_idle = done\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

/// Tests that on_idle = nudge sends a continue message then escalates when exhausted.
///
/// Flow: agent idles → nudge fires (1 attempt) → agent responds via turns →
/// agent idles again → nudge exhausted → escalate → Waiting.
#[test]
#[ignore = "BLOCKED BY: claudeless TUI does not process tmux send-keys input for multi-turn (less-nudge-input)"]
fn on_idle_nudge_sends_continue_message() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_nudge_then_done());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_idle_nudge(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // After the nudge fires and the agent responds, it idles again.
    // With Attempts::Finite(1), the nudge is exhausted and escalates to Waiting.
    let waiting = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("waiting")
    });
    assert!(
        waiting,
        "job should escalate to Waiting after nudge exhaustion\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

/// Tests on_idle gate with passing command advances the job.
///
/// Lifecycle: agent spawns → becomes idle (stop_reason: null) →
/// liveness timer fires → on_idle action triggers → gate command runs →
/// gate exits 0 → job advances to Completed.
#[test]
fn on_idle_gate_advances_when_command_passes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_idle_gate_pass(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(done, "job should complete via on_idle gate (exit 0)");
}

/// Tests on_idle gate with failing command escalates the job.
///
/// Lifecycle: agent spawns → becomes idle (stop_reason: null) →
/// liveness timer fires → on_idle action triggers → gate command runs →
/// gate exits non-zero → job escalates to Waiting.
#[test]
fn on_idle_gate_escalates_when_command_fails() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_idle_gate_fail(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let waiting = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("waiting")
    });
    assert!(
        waiting,
        "job should be in Waiting status after on_idle gate fails"
    );
}

// =============================================================================
// on_dead tests
// =============================================================================

/// Tests that on_dead = done completes the job when agent exits.
///
/// Uses claudeless -p (print mode) which exits immediately after one response.
/// The watcher detects the session death and triggers on_dead=done.
#[test]
fn on_dead_done_treats_exit_as_success() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_dead_done(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // claudeless -p exits immediately after one response, closing the tmux
    // session. The watcher detects this via liveness check and fires on_dead=done.
    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(
        done,
        "job should complete via on_dead=done after agent exit\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

/// Tests that on_dead = escalate sets the job to Waiting status.
///
/// Uses claudeless -p (print mode) which exits immediately after one response.
/// The watcher detects the session death and triggers on_dead=escalate.
#[test]
fn on_dead_escalate_sets_waiting_status() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_dead_escalate(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // claudeless -p exits immediately, closing the tmux session.
    // The watcher detects this and fires on_dead=escalate → Waiting.
    let waiting = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("waiting")
    });
    assert!(
        waiting,
        "job should be in Waiting status after on_dead=escalate"
    );
}

// =============================================================================
// on_error tests
// =============================================================================

#[test]
#[ignore = "BLOCKED BY: claudeless max_matches resets per-process; recover spawns new process causing infinite rate_limit loop (less-810230d6)"]
fn on_error_recover_retries_after_rate_limit() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_rate_limit());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_error_recover(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(done, "job should complete after rate limit recovery");
}

#[test]
fn on_error_escalate_on_network_failure() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_network_failure());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_error_recover(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let waiting = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("waiting")
    });
    assert!(waiting, "job should escalate after network errors");
}
