//! Integration tests for agent logs directory structure.
//!
//! Tests verify:
//! - Logs are written to `logs/agent/{job_id}/{step}.log`
//! - `oj agent logs <id>` retrieves all step logs
//! - `oj agent logs <id> --step <step>` retrieves a single step's log
//!
//! NOTE: Most tests require claudeless to write session JSONL files for log
//! entry extraction. Tests that depend on this are marked as ignored until
//! claudeless supports this feature.

use crate::prelude::*;

/// Scenario: agent makes tool calls that generate log entries.
fn scenario_with_tool_calls() -> &'static str {
    r#"
name = "tool-calls"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Working on the task."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "echo 'step done'" }

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#
}

/// Runbook with two agent steps: plan and implement.
fn multi_step_agent_runbook(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "plan"
run = {{ agent = "planner" }}
on_done = "implement"

[[job.build.step]]
name = "implement"
run = {{ agent = "implementer" }}

[agent.planner]
run = "claudeless --scenario {} -p"
prompt = "Create a plan."
on_dead = "done"
env = {{ OJ_STEP = "plan" }}

[agent.implementer]
run = "claudeless --scenario {} -p"
prompt = "Implement the plan."
on_dead = "done"
env = {{ OJ_STEP = "implement" }}
"#,
        scenario_path.display(),
        scenario_path.display()
    )
}

/// Tests that agent logs are written to the correct directory structure:
/// `logs/agent/{job_id}/{step}.log`
///
/// Lifecycle:
/// 1. Job with two agent steps (plan, implement) starts
/// 2. Each agent makes a Bash tool call (generating log entries)
/// 3. Job completes
/// 4. Verify log files exist at expected paths
#[test]
#[ignore = "BLOCKED BY: claudeless JSONL missing fields for log extraction (less-4afbd0cc)"]
fn agent_logs_written_to_job_step_structure() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_with_tool_calls());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = multi_step_agent_runbook(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // Wait for job to complete
    let done = wait_for(SPEC_WAIT_MAX_MS * 10, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(done, "job should complete");

    // Get the job ID from job list
    let list_output = temp.oj().args(&["job", "list"]).passes().stdout();
    // Extract job ID (first 8 chars of UUID in the list)
    let job_id = list_output
        .lines()
        .find(|l| l.contains("completed"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find job ID");

    // Verify log directory structure exists
    let logs_dir = temp.state_path().join("logs/agent").join(job_id);
    assert!(
        logs_dir.exists(),
        "agent logs directory should exist at {:?}",
        logs_dir
    );

    // Verify step log files exist
    let plan_log = logs_dir.join("plan.log");
    let implement_log = logs_dir.join("implement.log");

    assert!(plan_log.exists(), "plan.log should exist at {:?}", plan_log);
    assert!(
        implement_log.exists(),
        "implement.log should exist at {:?}",
        implement_log
    );

    // Verify logs have content (bash command should be logged)
    let plan_content = std::fs::read_to_string(&plan_log).unwrap();
    let impl_content = std::fs::read_to_string(&implement_log).unwrap();

    assert!(!plan_content.is_empty(), "plan.log should have content");
    assert!(
        !impl_content.is_empty(),
        "implement.log should have content"
    );
}

/// Tests `oj agent logs <id>` command succeeds for a completed job.
///
/// Note: With claudeless -p, no log entries are extracted (session JSONL not written),
/// so this test verifies the command works but may return empty content.
#[test]
fn agent_logs_command_succeeds_after_job_completes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_with_tool_calls());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = multi_step_agent_runbook(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let job_id = std::cell::RefCell::new(String::new());
    let done = wait_for(SPEC_WAIT_MAX_MS * 10, || {
        let out = temp
            .oj()
            .args(&["job", "list", "--output", "json"])
            .passes()
            .stdout();
        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&out) {
            if let Some(p) = list.iter().find(|p| p["step"] == "done") {
                *job_id.borrow_mut() = p["id"].as_str().unwrap().to_string();
                return true;
            }
        }
        false
    });
    assert!(done, "job should complete");
    let job_id = job_id.into_inner();

    // Test `oj agent logs <id>` succeeds (doesn't error)
    temp.oj().args(&["agent", "logs", &job_id]).passes();
}

/// Tests `oj agent logs <id> --step <step>` command succeeds for a completed job.
///
/// Note: With claudeless -p, no log entries are extracted (session JSONL not written),
/// so this test verifies the command works but may return empty content.
#[test]
fn agent_logs_command_with_step_filter_succeeds() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_with_tool_calls());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = multi_step_agent_runbook(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let job_id = std::cell::RefCell::new(String::new());
    let done = wait_for(SPEC_WAIT_MAX_MS * 10, || {
        let out = temp
            .oj()
            .args(&["job", "list", "--output", "json"])
            .passes()
            .stdout();
        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&out) {
            if let Some(p) = list.iter().find(|p| p["step"] == "done") {
                *job_id.borrow_mut() = p["id"].as_str().unwrap().to_string();
                return true;
            }
        }
        false
    });
    assert!(done, "job should complete");
    let job_id = job_id.into_inner();

    // Test `oj agent logs <id> --step plan` succeeds (doesn't error)
    temp.oj()
        .args(&["agent", "logs", &job_id, "--step", "plan"])
        .passes();

    // Test `oj agent logs <id> --step implement` succeeds (doesn't error)
    temp.oj()
        .args(&["agent", "logs", &job_id, "--step", "implement"])
        .passes();
}

/// Tests that `oj agent logs` with an invalid job ID returns an appropriate message.
#[test]
fn agent_logs_command_with_invalid_id() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    // Test `oj agent logs nonexistent` returns empty (no logs for that ID)
    temp.oj()
        .args(&["agent", "logs", "nonexistent-id"])
        .passes();
}
