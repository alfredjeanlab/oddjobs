//! ExitPlanMode decision tests using claudeless simulator.
//!
//! Tests the Plan decision flow when an agent calls ExitPlanMode:
//! - Decision source shows as "plan" (not "approval")
//! - Decision context displays the plan content
//! - Resolving with Accept (options 1-3) resumes the job
//! - Resolving with Cancel (option 5) cancels the job
//! - Resolving with Revise (option 4) sends feedback

use crate::prelude::*;

// =============================================================================
// Scenarios
// =============================================================================

/// Agent calls ExitPlanMode with plan content. No auto-approve, so it waits.
fn scenario_exit_plan_mode() -> &'static str {
    r##"
name = "plan-approval"

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "I'll create a plan for implementing the feature."

[[responses.response.tool_calls]]
tool = "ExitPlanMode"

[responses.response.tool_calls.input]
plan = "# Test Plan\n\n## Steps\n1. Add auth module\n2. Write tests"

[[responses]]
pattern = { type = "any" }
response = "Plan approved, proceeding."

[tool_execution]
mode = "live"
"##
}

// =============================================================================
// Runbooks
// =============================================================================

/// Runbook for plan tests where agent proceeds after approval and completes via
/// on_idle = "done". Used for Accept resolution tests.
fn runbook_plan_accept(scenario_path: &std::path::Path) -> String {
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

[agent.planner]
run = "claudeless --scenario {}"
prompt = "Create a plan for the feature."
on_idle = "done"
"#,
        scenario_path.display()
    )
}

/// Runbook for plan tests where agent waits at the plan approval dialog.
fn runbook_plan_wait(scenario_path: &std::path::Path) -> String {
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

[agent.planner]
run = "claudeless --scenario {}"
prompt = "Create a plan for the feature."
"#,
        scenario_path.display()
    )
}

// =============================================================================
// Tests: Decision Creation
// =============================================================================

/// Tests that ExitPlanMode creates a decision with "plan" source and plan content.
#[test]
fn exit_plan_mode_creates_plan_decision() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_exit_plan_mode());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_plan_wait(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // Wait for decision to be created with "plan" source
    let has_decision = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["decision", "list"])
            .passes()
            .stdout()
            .contains("plan")
    });
    assert!(
        has_decision,
        "decision should be created with plan source\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

/// Tests that the plan decision shows the plan content.
#[test]
fn plan_decision_shows_plan_content() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_exit_plan_mode());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_plan_wait(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let has_decision = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["decision", "list"])
            .passes()
            .stdout()
            .contains("plan")
    });
    assert!(
        has_decision,
        "decision should be created\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );

    let list_output = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id = extract_decision_id(&list_output)
        .expect("should be able to extract decision ID from list output");

    let show_output = temp
        .oj()
        .args(&["decision", "show", &decision_id])
        .passes()
        .stdout();

    // Verify plan content appears in context
    assert!(
        show_output.contains("Test Plan"),
        "decision show should contain plan content, got:\n{}\ndaemon log:\n{}",
        show_output,
        temp.daemon_log()
    );
    assert!(
        show_output.contains("Add auth module"),
        "decision show should contain plan steps, got:\n{}",
        show_output
    );
}

/// Tests that plan decision shows the 5 plan-specific options.
#[test]
fn plan_decision_shows_five_options() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_exit_plan_mode());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_plan_wait(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let has_decision = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["decision", "list"])
            .passes()
            .stdout()
            .contains("plan")
    });
    assert!(
        has_decision,
        "decision should be created\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );

    let list_output = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id = extract_decision_id(&list_output).expect("should extract decision ID");

    let show_output = temp
        .oj()
        .args(&["decision", "show", &decision_id])
        .passes()
        .stdout();

    assert!(
        show_output.contains("Accept (clear context)"),
        "should show Accept (clear context) option, got:\n{}",
        show_output
    );
    assert!(
        show_output.contains("Revise"),
        "should show Revise option, got:\n{}",
        show_output
    );
    assert!(
        show_output.contains("Cancel"),
        "should show Cancel option, got:\n{}",
        show_output
    );
}

// =============================================================================
// Tests: Decision Resolution
// =============================================================================

/// Tests that resolving with Cancel (option 5) cancels the job.
#[test]
fn resolve_plan_with_cancel_cancels_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_exit_plan_mode());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_plan_wait(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    let has_decision = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["decision", "list"])
            .passes()
            .stdout()
            .contains("plan")
    });
    assert!(
        has_decision,
        "decision should be created\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );

    let list_output = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id = extract_decision_id(&list_output).expect("should extract decision ID");

    // Option 5 = Cancel
    temp.oj()
        .args(&["decision", "resolve", &decision_id, "5"])
        .passes();

    // Job should be cancelled
    let cancelled = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("cancelled")
    });
    assert!(
        cancelled,
        "job should be cancelled after resolving with Cancel option\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

/// Tests that resolving a plan decision with Accept (option 1) sends key
/// presses to the agent session, causing it to proceed and complete the job.
///
/// Lifecycle: agent calls ExitPlanMode → decision created with source='plan' →
/// resolve with Accept (option 1) → daemon sends "Enter" to tmux session →
/// claudeless receives input, responds → agent idles → on_idle=done → job
/// completes.
#[test]
fn resolve_plan_with_accept_completes_job() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/scenarios/test.toml", scenario_exit_plan_mode());

    let scenario_path = temp.path().join(".oj/scenarios/test.toml");
    let runbook = runbook_plan_accept(&scenario_path);
    temp.file(".oj/runbooks/build.toml", &runbook);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // Wait for plan decision to be created
    let has_decision = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["decision", "list"])
            .passes()
            .stdout()
            .contains("plan")
    });
    assert!(
        has_decision,
        "decision should be created with plan source\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );

    let list_output = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id = extract_decision_id(&list_output).expect("should extract decision ID");

    // Option 1 = Accept (clear context) — sends "Enter" to session
    temp.oj()
        .args(&["decision", "resolve", &decision_id, "1"])
        .passes();

    // Agent should proceed after Accept and eventually complete via on_idle=done
    let completed = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });
    assert!(
        completed,
        "job should complete after Accept resolution\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

// =============================================================================
// Helpers
// =============================================================================

/// Extract the first decision ID from `oj decision list` output.
fn extract_decision_id(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("ID") || line.starts_with('-') {
            continue;
        }
        if let Some(id) = line.split_whitespace().next() {
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Some(id.to_string());
            }
        }
    }
    None
}
