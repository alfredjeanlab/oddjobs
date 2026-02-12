//! Tests for on_dead gate action behavior.
//!
//! Verifies the full lifecycle: agent spawns → agent process exits → on_dead
//! action triggers → gate command runs → job advances on exit 0 or
//! escalates on non-zero.

use crate::prelude::*;

/// Agent responds once and exits (for use with -p mode)
fn scenario_simple() -> &'static str {
    r#"
[claude]
trusted = true

[[responses]]
on = "*"
say = "Task complete."

[tools]
mode = "live"

[tools.Bash]
approve = true
"#
}

/// Runbook with on_dead gate that runs `true` (always exits 0).
/// Job should complete successfully.
fn runbook_gate_passes(scenario_path: &std::path::Path) -> String {
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
prompt = "Complete the task."
on_dead = {{ action = "gate", run = "true" }}
"#,
        scenario_path.display()
    )
}

/// Runbook with on_dead gate that runs `false` (always exits 1).
/// Job should escalate to Waiting status.
fn runbook_gate_fails(scenario_path: &std::path::Path) -> String {
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
prompt = "Complete the task."
on_dead = {{ action = "gate", run = "false" }}
"#,
        scenario_path.display()
    )
}

/// Runbook with on_dead gate that checks for a file the agent creates.
/// Demonstrates realistic gate usage: verify agent output before advancing.
fn runbook_gate_checks_output(scenario_path: &std::path::Path) -> String {
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
prompt = "Create the output file."
on_dead = {{ action = "gate", run = "test -f output.txt" }}
"#,
        scenario_path.display()
    )
}

/// Tests the full on_dead gate lifecycle with a passing gate command.
///
/// Lifecycle: agent spawns → agent exits → on_dead triggers → gate runs `true`
/// (exit 0) → job advances to Completed.
#[test]
fn on_dead_gate_exit_zero_advances_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/gate.toml", scenario_simple());

    let scenario_path = temp.path().join(".oj/scenarios/gate.toml");
    let runbook = runbook_gate_passes(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "gate-pass"]).passes();

    // claudeless -p exits immediately. The watcher detects session death
    // and runs the gate command. Exit 0 should advance the job.
    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(
        done,
        "job should complete when gate command exits 0\ndaemon log:\n{}",
        temp.daemon_log()
    );
}

/// Tests the full on_dead gate lifecycle with a failing gate command.
///
/// Lifecycle: agent spawns → agent exits → on_dead triggers → gate runs `false`
/// (exit 1) → job escalates to Waiting.
#[test]
fn on_dead_gate_nonzero_exit_escalates_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/gate.toml", scenario_simple());

    let scenario_path = temp.path().join(".oj/scenarios/gate.toml");
    let runbook = runbook_gate_fails(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "gate-fail"]).passes();

    // claudeless -p exits immediately. The watcher detects session death
    // and runs the gate command. Non-zero exit should escalate the job.
    let waiting = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("waiting")
    });
    assert!(
        waiting,
        "job should be in Waiting status when gate command exits non-zero\ndaemon log:\n{}",
        temp.daemon_log()
    );
}

/// Tests gate command verifying agent output exists.
///
/// This test uses a scenario where the agent creates output.txt via Bash tool.
/// The gate command `test -f output.txt` verifies the file was created before
/// advancing the job.
#[test]
fn on_dead_gate_verifies_agent_output() {
    let temp = Project::empty();
    temp.git_init();

    // Scenario where agent creates a file
    let scenario = r#"
[claude]
trusted = true

[[responses]]
on = "*"
say = "Creating output file."

[[responses.tools]]
call = "Bash"
input = { command = "echo 'result' > output.txt" }

[tools]
mode = "live"

[tools.Bash]
approve = true
"#;

    temp.file(".oj/scenarios/creates-output.toml", scenario);
    let scenario_path = temp.path().join(".oj/scenarios/creates-output.toml");
    let runbook = runbook_gate_checks_output(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "verify-output"]).passes();

    // Agent creates output.txt, then exits. Gate checks file exists.
    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(
        done,
        "job should complete when gate verifies agent output\ndaemon log:\n{}",
        temp.daemon_log()
    );
}
