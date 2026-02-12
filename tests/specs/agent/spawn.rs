//! Agent spawn and execution tests using claudeless simulator.
//!
//! Tests the Effect::SpawnAgent -> LocalAdapter::spawn() path triggered by
//! `run = { agent = "..." }` directives.

use crate::prelude::*;

/// Generate a scenario with configurable trust setting
fn scenario(trusted: bool) -> String {
    format!(
        r#"
[claude]
trusted = {trusted}

[[responses]]
on = "*"
say = "Task complete."

[tools]
mode = "live"

[tools.Bash]
approve = true
"#,
    )
}

/// Runbook using -p (print/non-interactive) mode. Agent exits after one response.
fn agent_runbook_print(scenario_path: &std::path::Path) -> String {
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
on_dead = "done"
env = {{ OJ_STEP = "work" }}
"#,
        scenario_path.display()
    )
}

/// Runbook using interactive mode (no -p). Agent stays alive and idles.
fn agent_runbook_interactive(scenario_path: &std::path::Path) -> String {
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
prompt = "Complete the task."
on_idle = "done"
env = {{ OJ_STEP = "work" }}
"#,
        scenario_path.display()
    )
}

/// Verifies the agent spawn flow with -p (print) mode:
/// - Job starts and reaches agent step
/// - Effect::SpawnAgent creates coop process via LocalAdapter
/// - Workspace is prepared with CLAUDE.md
/// - Agent (claudeless -p) runs and exits after one response
/// - on_dead = "done" advances the job
#[test]
fn agent_spawn_creates_session_and_completes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/spawn.toml", &scenario(true));

    let scenario_path = temp.path().join(".oj/scenarios/spawn.toml");
    let runbook = agent_runbook_print(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "spawn-test"]).passes();

    // claudeless -p exits immediately. Detection requires liveness poll + deferred timer.
    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "job should complete via agent spawn path");

    temp.oj().args(&["job", "list"]).passes().stdout_has("completed");
}

/// Verifies the agent spawn flow with interactive mode (no -p):
/// - Agent stays alive and idles after responding
/// - on_idle = "done" advances the job
#[test]
fn agent_spawn_interactive_idle_completes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/spawn.toml", &scenario(true));

    let scenario_path = temp.path().join(".oj/scenarios/spawn.toml");
    let runbook = agent_runbook_interactive(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "spawn-test"]).passes();

    let done = wait_for(SPEC_AGENT_WAIT_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "job should complete via on_idle = done");
}

/// Tests multi-step job: shell init → agent plan (gate) → agent implement (gate) → done.
///
/// Each agent uses on_dead = gate with a shell check command. The gate verifies
/// the agent's work (file creation) before advancing to the next step.
#[test]
fn multi_step_job_with_gates_completes() {
    let temp = Project::empty();
    temp.git_init();

    // Plan agent: creates output/plan.txt via tool call
    let plan_scenario = r#"
[claude]
trusted = true

[[responses]]
on = "*"
say = "Creating the plan document."

[[responses.tools]]
call = "Bash"
input = { command = "mkdir -p output && echo '# Test Plan' > output/plan.txt" }

[tools]
mode = "live"

[tools.Bash]
approve = true
"#;

    // Implement agent: creates output/impl.txt via tool call
    let impl_scenario = r#"
[claude]
trusted = true

[[responses]]
on = "*"
say = "Implementation complete."

[[responses.tools]]
call = "Bash"
input = { command = "echo '# Implementation' > output/impl.txt" }

[tools]
mode = "live"

[tools.Bash]
approve = true
"#;

    temp.file(".oj/scenarios/plan.toml", plan_scenario);
    temp.file(".oj/scenarios/impl.toml", impl_scenario);

    let plan_path = temp.path().join(".oj/scenarios/plan.toml");
    let impl_path = temp.path().join(".oj/scenarios/impl.toml");

    let runbook = format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "init"
run = "mkdir -p output"
on_done = {{ step = "plan" }}

[[job.build.step]]
name = "plan"
run = {{ agent = "planner" }}
on_done = {{ step = "implement" }}

[[job.build.step]]
name = "implement"
run = {{ agent = "implementer" }}

[agent.planner]
run = "claudeless --scenario {plan} -p"
prompt = "Create a plan."
on_dead = {{ action = "gate", run = "test -f output/plan.txt" }}

[agent.implementer]
run = "claudeless --scenario {impl} -p"
prompt = "Implement the plan."
on_dead = {{ action = "gate", run = "test -f output/impl.txt" }}
"#,
        plan = plan_path.display(),
        impl = impl_path.display()
    );

    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "e2e-test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS * 10, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "multi-step job should complete via gates");
}

/// Tests that trusted scenario completes without the trust prompt handler blocking.
///
/// With `OJ_PROMPT_POLL_MS` set low and `trusted = true`, the spawn-time prompt
/// handler exits quickly and the agent proceeds without delay.
#[test]
fn agent_spawn_graceful_when_no_trust_prompt() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/spawn.toml", &scenario(true));

    let scenario_path = temp.path().join(".oj/scenarios/spawn.toml");
    let runbook = agent_runbook_print(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "trust-test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "job should complete - no trust prompt shown for trusted scenario");
}

/// Tests that the bypass permissions prompt is auto-accepted.
///
/// When Claude Code runs with --dangerously-skip-permissions, it shows:
/// ```text
/// WARNING: Claude Code running in Bypass Permissions mode
/// ...
/// ❯ 1. No, exit
///   2. Yes, I accept
/// ```
///
/// The adapter should detect this and send "2" to accept.
#[test]
fn agent_spawn_auto_accepts_bypass_permissions_prompt() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/spawn.toml", &scenario(true));

    let scenario_path = temp.path().join(".oj/scenarios/spawn.toml");
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
run = "claudeless --scenario {} --dangerously-skip-permissions"
prompt = "Complete the task."
on_idle = "done"
"#,
        scenario_path.display()
    );
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "bypass-test"]).passes();

    let done = wait_for(SPEC_AGENT_WAIT_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "job should complete - bypass permissions prompt was auto-accepted");
}

/// Tests that untrusted scenario completes via auto-accept of the trust prompt.
///
/// Claudeless shows a trust dialog when `trusted = false`. The watcher's
/// `check_and_accept_trust_prompt` detects "Do you trust" and sends "y".
/// Uses `-p` mode so the agent exits after one response and `on_dead = "done"`.
#[test]
fn agent_spawn_auto_accepts_trust_prompt() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/spawn.toml", &scenario(false));

    let scenario_path = temp.path().join(".oj/scenarios/spawn.toml");
    let runbook = agent_runbook_print(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "trust-test"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(done, "job should complete - trust prompt was auto-acknowledged");
}
