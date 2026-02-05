//! Multi-agent concurrency specs
//!
//! Verify that multiple agents can run simultaneously, complete independently,
//! and that one agent's failure doesn't block others.

use crate::prelude::*;

/// Fast scenario that completes immediately
fn fast_scenario(name: &str) -> String {
    format!(
        r#"
name = "{name}"
trusted = true

[[responses]]
pattern = {{ type = "any" }}

[responses.response]
text = "Agent {name} complete."

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#
    )
}

/// Blocking scenario that keeps agent alive via sleep
fn blocking_scenario(name: &str) -> String {
    format!(
        r#"
name = "{name}"
trusted = true

[[responses]]
pattern = {{ type = "any" }}

[responses.response]
text = "Starting blocking work."

[[responses.response.tool_calls]]
tool = "Bash"
input = {{ command = "sleep 30" }}

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#
    )
}

/// Scenario that fails immediately
fn failing_scenario(name: &str) -> String {
    format!(
        r#"
name = "{name}"
trusted = true

[[responses]]
pattern = {{ type = "any" }}

[responses.response]
text = "About to fail."

[[responses.response.tool_calls]]
tool = "Bash"
input = {{ command = "exit 1" }}

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#
    )
}

// =============================================================================
// Test 1: Multiple agents from different jobs run simultaneously
// =============================================================================

#[test]
fn multiple_agents_run_simultaneously() {
    let temp = Project::empty();
    temp.git_init();

    // Create scenarios for two agents
    temp.file(".oj/scenarios/alpha.toml", &fast_scenario("alpha"));
    temp.file(".oj/scenarios/beta.toml", &fast_scenario("beta"));

    let alpha_path = temp.path().join(".oj/scenarios/alpha.toml");
    let beta_path = temp.path().join(".oj/scenarios/beta.toml");

    // Runbook with two independent commands that each spawn an agent
    let runbook = format!(
        r#"
[command.alpha]
run = {{ job = "alpha_job" }}

[command.beta]
run = {{ job = "beta_job" }}

[job.alpha_job]
[[job.alpha_job.step]]
name = "work"
run = {{ agent = "alpha_agent" }}

[job.beta_job]
[[job.beta_job.step]]
name = "work"
run = {{ agent = "beta_agent" }}

[agent.alpha_agent]
run = "claudeless --scenario {alpha}"
prompt = "Complete alpha task."
on_idle = "done"

[agent.beta_agent]
run = "claudeless --scenario {beta}"
prompt = "Complete beta task."
on_idle = "done"
"#,
        alpha = alpha_path.display(),
        beta = beta_path.display()
    );

    temp.file(".oj/runbooks/multi.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();

    // Start both jobs
    temp.oj().args(&["run", "alpha"]).passes();
    temp.oj().args(&["run", "beta"]).passes();

    // Wait for both jobs to complete
    let both_done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.matches("completed").count() >= 2
    });

    if !both_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        both_done,
        "both agent jobs should complete, proving multiple agents can run"
    );

    // Verify both jobs completed
    let job_output = temp.oj().args(&["job", "list"]).passes().stdout();
    assert!(
        job_output.contains("alpha_job") && job_output.contains("beta_job"),
        "should see both jobs in list:\n{}",
        job_output
    );
}

// =============================================================================
// Test 2: Three agents run concurrently and complete independently
// =============================================================================

#[test]
fn three_agents_complete_independently() {
    let temp = Project::empty();
    temp.git_init();

    // Create scenarios for three agents
    temp.file(".oj/scenarios/agent1.toml", &fast_scenario("agent1"));
    temp.file(".oj/scenarios/agent2.toml", &fast_scenario("agent2"));
    temp.file(".oj/scenarios/agent3.toml", &fast_scenario("agent3"));

    let path1 = temp.path().join(".oj/scenarios/agent1.toml");
    let path2 = temp.path().join(".oj/scenarios/agent2.toml");
    let path3 = temp.path().join(".oj/scenarios/agent3.toml");

    let runbook = format!(
        r#"
[command.job1]
run = {{ job = "job1" }}

[command.job2]
run = {{ job = "job2" }}

[command.job3]
run = {{ job = "job3" }}

[job.job1]
[[job.job1.step]]
name = "work"
run = {{ agent = "agent1" }}

[job.job2]
[[job.job2.step]]
name = "work"
run = {{ agent = "agent2" }}

[job.job3]
[[job.job3.step]]
name = "work"
run = {{ agent = "agent3" }}

[agent.agent1]
run = "claudeless --scenario {path1}"
prompt = "Task 1."
on_idle = "done"

[agent.agent2]
run = "claudeless --scenario {path2}"
prompt = "Task 2."
on_idle = "done"

[agent.agent3]
run = "claudeless --scenario {path3}"
prompt = "Task 3."
on_idle = "done"
"#,
        path1 = path1.display(),
        path2 = path2.display(),
        path3 = path3.display()
    );

    temp.file(".oj/runbooks/triple.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();

    // Start all three jobs
    temp.oj().args(&["run", "job1"]).passes();
    temp.oj().args(&["run", "job2"]).passes();
    temp.oj().args(&["run", "job3"]).passes();

    // Wait for all three to complete
    let all_done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.matches("completed").count() >= 3
    });

    if !all_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        all_done,
        "all three agent jobs should complete independently"
    );
}

// =============================================================================
// Test 3: Blocking agents can run alongside fast agents
// =============================================================================

#[test]
fn blocking_and_fast_agents_coexist() {
    let temp = Project::empty();
    temp.git_init();

    // One blocking agent, one fast agent
    temp.file(".oj/scenarios/blocker.toml", &blocking_scenario("blocker"));
    temp.file(".oj/scenarios/fast.toml", &fast_scenario("fast"));

    let blocker_path = temp.path().join(".oj/scenarios/blocker.toml");
    let fast_path = temp.path().join(".oj/scenarios/fast.toml");

    let runbook = format!(
        r#"
[command.blocking]
run = {{ job = "blocking_job" }}

[command.fast]
run = {{ job = "fast_job" }}

[job.blocking_job]
[[job.blocking_job.step]]
name = "work"
run = {{ agent = "blocker" }}

[job.fast_job]
[[job.fast_job.step]]
name = "work"
run = {{ agent = "fast" }}

[agent.blocker]
run = "claudeless --scenario {blocker}"
prompt = "Block for a while."
on_idle = "done"

[agent.fast]
run = "claudeless --scenario {fast}"
prompt = "Complete quickly."
on_idle = "done"
"#,
        blocker = blocker_path.display(),
        fast = fast_path.display()
    );

    temp.file(".oj/runbooks/mixed.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();

    // Start blocking agent first, then fast agent
    temp.oj().args(&["run", "blocking"]).passes();
    temp.oj().args(&["run", "fast"]).passes();

    // Fast agent should complete while blocking agent is still running
    let fast_done_blocking_running = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        // Fast job completed, blocking job still running
        out.contains("completed") && out.contains("running")
    });

    if !fast_done_blocking_running {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        fast_done_blocking_running,
        "fast agent should complete while blocking agent runs (independent execution)"
    );
}

// =============================================================================
// Test 4: Failed agent doesn't block other agents
// =============================================================================

#[test]
fn failed_agent_does_not_block_others() {
    let temp = Project::empty();
    temp.git_init();

    // One failing agent, one succeeding agent
    temp.file(".oj/scenarios/fail.toml", &failing_scenario("fail"));
    temp.file(".oj/scenarios/succeed.toml", &fast_scenario("succeed"));

    let fail_path = temp.path().join(".oj/scenarios/fail.toml");
    let succeed_path = temp.path().join(".oj/scenarios/succeed.toml");

    let runbook = format!(
        r#"
[command.failing]
run = {{ job = "failing_job" }}

[command.succeeding]
run = {{ job = "succeeding_job" }}

[job.failing_job]
[[job.failing_job.step]]
name = "work"
run = {{ agent = "fail_agent" }}

[job.succeeding_job]
[[job.succeeding_job.step]]
name = "work"
run = {{ agent = "succeed_agent" }}

[agent.fail_agent]
run = "claudeless --scenario {fail} -p"
prompt = "This will fail."
on_dead = "fail"

[agent.succeed_agent]
run = "claudeless --scenario {succeed}"
prompt = "This will succeed."
on_idle = "done"
"#,
        fail = fail_path.display(),
        succeed = succeed_path.display()
    );

    temp.file(".oj/runbooks/failsafe.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();

    // Start both jobs
    temp.oj().args(&["run", "failing"]).passes();
    temp.oj().args(&["run", "succeeding"]).passes();

    // Succeeding job should complete regardless of failing job
    let succeed_done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !succeed_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        succeed_done,
        "succeeding agent should complete even when another agent fails"
    );

    // Verify both final states
    let job_output = temp.oj().args(&["job", "list"]).passes().stdout();
    assert!(
        job_output.contains("completed"),
        "succeeding job should be completed:\n{}",
        job_output
    );
    assert!(
        job_output.contains("failed"),
        "failing job should be failed:\n{}",
        job_output
    );
}

// =============================================================================
// Test 5: Rapid agent starts and completions work correctly
// =============================================================================

#[test]
fn rapid_agent_completions_work() {
    let temp = Project::empty();
    temp.git_init();

    // Create scenarios for four fast agents
    for i in 1..=4 {
        temp.file(
            &format!(".oj/scenarios/rapid{}.toml", i),
            &fast_scenario(&format!("rapid{}", i)),
        );
    }

    let paths: Vec<_> = (1..=4)
        .map(|i| temp.path().join(format!(".oj/scenarios/rapid{}.toml", i)))
        .collect();

    let runbook = format!(
        r#"
[command.rapid1]
run = {{ job = "rapid1_job" }}

[command.rapid2]
run = {{ job = "rapid2_job" }}

[command.rapid3]
run = {{ job = "rapid3_job" }}

[command.rapid4]
run = {{ job = "rapid4_job" }}

[job.rapid1_job]
[[job.rapid1_job.step]]
name = "work"
run = {{ agent = "rapid1" }}

[job.rapid2_job]
[[job.rapid2_job.step]]
name = "work"
run = {{ agent = "rapid2" }}

[job.rapid3_job]
[[job.rapid3_job.step]]
name = "work"
run = {{ agent = "rapid3" }}

[job.rapid4_job]
[[job.rapid4_job.step]]
name = "work"
run = {{ agent = "rapid4" }}

[agent.rapid1]
run = "claudeless --scenario {path1}"
prompt = "Rapid task 1."
on_idle = "done"

[agent.rapid2]
run = "claudeless --scenario {path2}"
prompt = "Rapid task 2."
on_idle = "done"

[agent.rapid3]
run = "claudeless --scenario {path3}"
prompt = "Rapid task 3."
on_idle = "done"

[agent.rapid4]
run = "claudeless --scenario {path4}"
prompt = "Rapid task 4."
on_idle = "done"
"#,
        path1 = paths[0].display(),
        path2 = paths[1].display(),
        path3 = paths[2].display(),
        path4 = paths[3].display()
    );

    temp.file(".oj/runbooks/rapid.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();

    // Start all four jobs in rapid succession
    temp.oj().args(&["run", "rapid1"]).passes();
    temp.oj().args(&["run", "rapid2"]).passes();
    temp.oj().args(&["run", "rapid3"]).passes();
    temp.oj().args(&["run", "rapid4"]).passes();

    // All four should complete
    let all_done = wait_for(SPEC_WAIT_MAX_MS * 10, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.matches("completed").count() >= 4
    });

    if !all_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        all_done,
        "all four rapidly-started agent jobs should complete"
    );
}
