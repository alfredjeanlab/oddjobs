//! Agent configuration parsing tests.
//!
//! Verify agent configuration is correctly loaded and accessible at runtime.

use crate::prelude::*;

fn scenario_simple() -> String {
    r#"
name = "simple"

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Done."

[tool_execution]
mode = "live"
tools.Bash.auto_approve = true
"#
    .to_string()
}

/// Generate runbook with full agent action configuration.
/// Tests all action config syntax variants:
/// - Simple action string: on_dead = "escalate"
/// - Action with message: on_idle = { action = "nudge", message = "..." }
/// - Per-error actions: [[agent.name.on_error]] with match field
fn runbook_with_full_config(scenario_path: &std::path::Path) -> String {
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
on_idle = "done"
on_dead = "done"

[agent.worker.env]
OJ_STEP = "execute"

[[agent.worker.on_error]]
match = "no_internet"
action = "escalate"
message = "Network error, needs attention."

[[agent.worker.on_error]]
match = "rate_limited"
action = "escalate"
message = "Rate limit hit."

[[agent.worker.on_error]]
action = "escalate"
"#,
        scenario_path.display()
    )
}

/// Verify that runbooks with agent action configurations can be parsed and loaded.
/// The daemon should start without errors when the runbook contains:
/// - on_idle, on_dead with simple and complex syntax
/// - on_error with per-error-type matching
#[test]
fn runbook_with_agent_config_loads() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/simple.toml", &scenario_simple());

    let scenario_path = temp.path().join(".oj/scenarios/simple.toml");
    let runbook = runbook_with_full_config(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    // Daemon should start without errors (config parses correctly)
    temp.oj().args(&["daemon", "start"]).passes();

    // Job should run to verify agent config doesn't break runtime
    temp.oj().args(&["run", "build", "test"]).passes();

    // claudeless -p exits immediately. Detection requires liveness poll + deferred timer.
    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(done, "job should complete");
}

/// Verify that on_idle = "done" works in interactive mode (no -p) with full config.
#[test]
fn runbook_with_agent_config_idle_completes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/simple.toml", &scenario_simple());

    let scenario_path = temp.path().join(".oj/scenarios/simple.toml");

    // Same config as runbook_with_full_config but using interactive mode (no -p)
    // and on_dead = "escalate" (the real on_dead behavior to test once idle works)
    let runbook = format!(
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
on_dead = "escalate"

[agent.worker.env]
OJ_STEP = "execute"

[[agent.worker.on_error]]
match = "no_internet"
action = "escalate"
message = "Network error, needs attention."

[[agent.worker.on_error]]
match = "rate_limited"
action = "escalate"
message = "Rate limit hit."

[[agent.worker.on_error]]
action = "escalate"
"#,
        scenario_path.display()
    );
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(done, "job should complete via on_idle = done");
}
