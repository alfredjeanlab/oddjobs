//! Agent hook tests for stop hook behavior.
//!
//! Verifies that the stop hook mechanism works: when coop blocks an agent's
//! exit attempt, the daemon receives `agent:stop:blocked` and dispatches
//! the on_idle action (e.g., done for job agents, escalate for standalone).

use crate::prelude::*;

/// Claudeless scenario: agent works then idles (triggering on_idle = done).
fn scenario_work_then_idle() -> &'static str {
    r#"
[claude]
trusted = true

[[responses]]
on = "*"
say = "Work complete."

[tools]
mode = "live"
"#
}

/// Runbook that uses on_idle = done to auto-complete.
///
/// Lifecycle: agent spawns → works → idles → on_idle fires → job advances.
fn runbook_on_idle_done(scenario_path: &std::path::Path) -> String {
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
on_done = {{ step = "finish" }}

[[job.build.step]]
name = "finish"
run = "echo done"

[agent.worker]
run = "claudeless --scenario {}"
prompt = "Do the work."
on_idle = "done"
"#,
        scenario_path.display()
    )
}

/// Tests that an agent completing via on_idle = done properly advances the job.
///
/// This exercises the core agent lifecycle path that replaces the old
/// `oj emit agent:signal complete` mechanism.
#[test]
fn agent_on_idle_done_advances_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/work.toml", scenario_work_then_idle());

    let scenario_path = temp.path().join(".oj/scenarios/work.toml");
    let runbook = runbook_on_idle_done(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "idle-test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(
        done,
        "job should complete via on_idle=done\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}
