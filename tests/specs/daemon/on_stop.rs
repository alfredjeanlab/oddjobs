//! Agent on_stop lifecycle handler specs.
//!
//! Verify that the on_stop config is written at spawn time and that
//! the correct default is applied based on context (job vs standalone).

use crate::prelude::*;

// =============================================================================
// Scenarios
// =============================================================================

/// Agent stops at end_turn (no tool calls) — triggers on_idle
fn scenario_end_turn() -> &'static str {
    r#"
name = "end-turn"

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "I've analyzed the task and here's my response."
"#
}

// =============================================================================
// Runbooks
// =============================================================================

fn runbook_default_on_stop(scenario_path: &std::path::Path) -> String {
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

fn runbook_explicit_on_stop_idle(scenario_path: &std::path::Path) -> String {
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
on_stop = "idle"
on_idle = "done"
"#,
        scenario_path.display()
    )
}

fn runbook_explicit_on_stop_escalate(scenario_path: &std::path::Path) -> String {
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
on_stop = "escalate"
on_idle = "done"
"#,
        scenario_path.display()
    )
}

/// Find the first config.json under {state_dir}/agents/ and return its contents.
fn read_agent_config(temp: &Project) -> Option<String> {
    let agents_dir = temp.state_path().join("agents");
    if !agents_dir.exists() {
        return None;
    }
    for entry in std::fs::read_dir(&agents_dir).ok()? {
        let entry = entry.ok()?;
        let config_path = entry.path().join("config.json");
        if config_path.exists() {
            return std::fs::read_to_string(&config_path).ok();
        }
    }
    None
}

// =============================================================================
// Tests: config.json written at spawn time
// =============================================================================

/// Job agent with no explicit on_stop should default to "signal".
#[test]
fn job_agent_default_on_stop_is_signal() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    temp.file(
        ".oj/runbooks/build.toml",
        &runbook_default_on_stop(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // Wait for agent to be spawned (job reaches running status)
    let running = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("running") || out.contains("completed")
    });
    assert!(
        running,
        "job should reach running or completed\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Verify config.json was written with on_stop = signal
    let config_found = wait_for(SPEC_WAIT_MAX_MS, || read_agent_config(&temp).is_some());
    assert!(config_found, "config.json should be written at spawn time");

    let config = read_agent_config(&temp).unwrap();
    assert!(
        config.contains("\"on_stop\":\"signal\""),
        "job agent should default to on_stop=signal, got: {}",
        config
    );
}

/// Job agent with explicit on_stop = "idle" should write idle to config.
#[test]
fn job_agent_explicit_on_stop_idle() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    temp.file(
        ".oj/runbooks/build.toml",
        &runbook_explicit_on_stop_idle(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let running = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("running") || out.contains("completed")
    });
    assert!(
        running,
        "job should reach running or completed\ndaemon log:\n{}",
        temp.daemon_log()
    );

    let config_found = wait_for(SPEC_WAIT_MAX_MS, || read_agent_config(&temp).is_some());
    assert!(config_found, "config.json should be written at spawn time");

    let config = read_agent_config(&temp).unwrap();
    assert!(
        config.contains("\"on_stop\":\"idle\""),
        "explicit on_stop=idle should be written to config, got: {}",
        config
    );
}

/// Job agent with explicit on_stop = "escalate" should write escalate to config.
#[test]
fn job_agent_explicit_on_stop_escalate() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    temp.file(
        ".oj/runbooks/build.toml",
        &runbook_explicit_on_stop_escalate(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let running = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("running") || out.contains("completed")
    });
    assert!(
        running,
        "job should reach running or completed\ndaemon log:\n{}",
        temp.daemon_log()
    );

    let config_found = wait_for(SPEC_WAIT_MAX_MS, || read_agent_config(&temp).is_some());
    assert!(config_found, "config.json should be written at spawn time");

    let config = read_agent_config(&temp).unwrap();
    assert!(
        config.contains("\"on_stop\":\"escalate\""),
        "explicit on_stop=escalate should be written to config, got: {}",
        config
    );
}

/// on_stop = idle with on_idle = done should still allow the job to
/// complete via normal idle detection (the on_stop config only affects the
/// Claude Code Stop hook, which doesn't fire in claudeless).
#[test]
fn on_stop_idle_does_not_interfere_with_on_idle() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    temp.file(
        ".oj/runbooks/build.toml",
        &runbook_explicit_on_stop_idle(&scenario_path),
    );

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
        "job should complete via on_idle=done (on_stop=idle should not interfere)\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}
