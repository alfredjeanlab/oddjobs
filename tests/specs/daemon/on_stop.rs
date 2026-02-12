//! Agent on_idle → coop stop mode mapping specs.
//!
//! Verify that the on_idle action is written to agent-config.json at spawn
//! time and that the correct coop stop mode is applied.

use crate::prelude::*;

/// Agent stops at end_turn (no tool calls) — triggers on_idle
fn scenario_end_turn() -> &'static str {
    r#"
[[responses]]
on = "*"
say = "I've analyzed the task and here's my response."
"#
}

fn runbook_on_idle_done(scenario_path: &std::path::Path) -> String {
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

fn runbook_on_idle_escalate(scenario_path: &std::path::Path) -> String {
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
on_idle = "escalate"
"#,
        scenario_path.display()
    )
}

fn runbook_on_idle_nudge(scenario_path: &std::path::Path) -> String {
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

/// Find the first agent-config.json under {state_dir}/agents/ and return its contents.
fn read_agent_config(temp: &Project) -> Option<String> {
    let agents_dir = temp.state_path().join("agents");
    if !agents_dir.exists() {
        return None;
    }
    for entry in std::fs::read_dir(&agents_dir).ok()? {
        let entry = entry.ok()?;
        let config_path = entry.path().join("agent-config.json");
        if config_path.exists() {
            return std::fs::read_to_string(&config_path).ok();
        }
    }
    None
}

/// Job agent with on_idle=done should use coop allow mode (no interception).
#[test]
fn job_agent_on_idle_done_is_allow() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    temp.file(".oj/runbooks/build.toml", &runbook_on_idle_done(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // agent-config.json is written at agent spawn time — wait for it directly
    let config_found = wait_for(SPEC_WAIT_MAX_MS * 4, || read_agent_config(&temp).is_some());
    assert!(
        config_found,
        "agent-config.json should be written at spawn time\ndaemon log:\n{}",
        temp.daemon_log()
    );

    let config = read_agent_config(&temp).unwrap();
    assert!(
        config.contains("\"mode\": \"allow\""),
        "on_idle=done should map to stop.mode=allow, got: {}",
        config
    );
}

/// Job agent with on_idle=escalate should use coop gate mode.
#[test]
fn job_agent_on_idle_escalate_is_gate() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    temp.file(".oj/runbooks/build.toml", &runbook_on_idle_escalate(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let config_found = wait_for(SPEC_WAIT_MAX_MS * 4, || read_agent_config(&temp).is_some());
    assert!(
        config_found,
        "agent-config.json should be written at spawn time\ndaemon log:\n{}",
        temp.daemon_log()
    );

    let config = read_agent_config(&temp).unwrap();
    assert!(
        config.contains("\"mode\": \"gate\""),
        "on_idle=escalate should map to stop.mode=gate, got: {}",
        config
    );
}

/// Job agent with on_idle=nudge should use coop gate mode.
#[test]
fn job_agent_on_idle_nudge_is_gate() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    temp.file(".oj/runbooks/build.toml", &runbook_on_idle_nudge(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let config_found = wait_for(SPEC_WAIT_MAX_MS * 4, || read_agent_config(&temp).is_some());
    assert!(
        config_found,
        "agent-config.json should be written at spawn time\ndaemon log:\n{}",
        temp.daemon_log()
    );

    let config = read_agent_config(&temp).unwrap();
    assert!(
        config.contains("\"mode\": \"gate\""),
        "on_idle=nudge should map to stop.mode=gate, got: {}",
        config
    );
}

/// Job with on_idle=done should complete normally via coop's allow mode.
#[test]
fn on_idle_done_completes_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_end_turn());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    temp.file(".oj/runbooks/build.toml", &runbook_on_idle_done(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let done = wait_for(SPEC_AGENT_WAIT_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(
        done,
        "job should complete via on_idle=done\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}
