//! Pipeline wait specs
//!
//! Verify `oj pipeline wait` works with single and multiple IDs.

use crate::prelude::*;

/// Runbook with two pipelines: one succeeds, one fails.
const WAIT_RUNBOOK: &str = r#"
[command.succeed]
run = { pipeline = "succeed" }

[pipeline.succeed]
[[pipeline.succeed.step]]
name = "execute"
run = "echo ok"

[command.fail_cmd]
run = { pipeline = "fail_cmd" }

[pipeline.fail_cmd]
[[pipeline.fail_cmd.step]]
name = "execute"
run = "exit 1"
"#;

fn setup() -> Project {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/wait.toml", WAIT_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();
    temp
}

/// Extract the pipeline ID from `oj pipeline list` output by matching a line containing the name.
fn extract_pipeline_id(temp: &Project, name_filter: &str) -> String {
    let output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    output
        .lines()
        .find(|l| l.contains(name_filter))
        .unwrap_or_else(|| {
            panic!(
                "no pipeline matching '{}' in output:\n{}",
                name_filter, output
            )
        })
        .split_whitespace()
        .next()
        .expect("should have an ID column")
        .to_string()
}

/// Extract all pipeline IDs from `oj pipeline list` output matching a name.
fn extract_pipeline_ids(temp: &Project, name_filter: &str) -> Vec<String> {
    let output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    output
        .lines()
        .filter(|l| l.contains(name_filter))
        .filter_map(|l| l.split_whitespace().next())
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn wait_single_pipeline_succeeds() {
    let temp = setup();
    temp.oj().args(&["run", "succeed"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("Completed")
    });
    assert!(done, "pipeline should complete");

    let id = extract_pipeline_id(&temp, "succeed");
    temp.oj()
        .args(&["pipeline", "wait", &id])
        .env("OJ_WAIT_POLL_MS", "10")
        .passes()
        .stdout_has("completed");
}

#[test]
fn wait_single_pipeline_failed_exits_nonzero() {
    let temp = setup();
    temp.oj().args(&["run", "fail_cmd"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("failed")
    });
    assert!(done, "pipeline should fail");

    let id = extract_pipeline_id(&temp, "fail_cmd");
    temp.oj()
        .args(&["pipeline", "wait", &id])
        .env("OJ_WAIT_POLL_MS", "10")
        .fails()
        .stderr_has("failed");
}

#[test]
fn wait_not_found_exits_nonzero() {
    let temp = setup();
    temp.oj()
        .args(&["pipeline", "wait", "nonexistent-id-12345"])
        .env("OJ_WAIT_POLL_MS", "10")
        .fails()
        .stderr_has("Pipeline not found");
}

#[test]
fn wait_multiple_ids_any_mode() {
    let temp = setup();
    temp.oj().args(&["run", "succeed"]).passes();
    temp.oj().args(&["run", "succeed"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.matches("Completed").count() >= 2
    });
    assert!(done, "both pipelines should complete");

    let ids = extract_pipeline_ids(&temp, "succeed");
    assert!(ids.len() >= 2, "should have at least 2 pipelines");

    // Wait for any — should succeed immediately since both are done
    temp.oj()
        .args(&["pipeline", "wait", &ids[0], &ids[1]])
        .env("OJ_WAIT_POLL_MS", "10")
        .passes()
        .stdout_has("completed");
}

#[test]
fn wait_multiple_ids_all_mode() {
    let temp = setup();
    temp.oj().args(&["run", "succeed"]).passes();
    temp.oj().args(&["run", "succeed"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.matches("Completed").count() >= 2
    });
    assert!(done, "both pipelines should complete");

    let ids = extract_pipeline_ids(&temp, "succeed");
    assert!(ids.len() >= 2, "should have at least 2 pipelines");

    // Wait for all — should print both
    let result = temp
        .oj()
        .args(&["pipeline", "wait", "--all", &ids[0], &ids[1]])
        .env("OJ_WAIT_POLL_MS", "10")
        .passes();

    // Both should be mentioned in output (match final "Pipeline ... completed" lines,
    // not step progress lines like "execute completed (0s)")
    let stdout = result.stdout();
    let pipeline_completed_count = stdout
        .lines()
        .filter(|l| l.starts_with("Pipeline") && l.contains("completed"))
        .count();
    assert_eq!(
        pipeline_completed_count, 2,
        "should report both pipelines as completed, got: {}",
        stdout
    );
}

#[test]
fn wait_all_mode_mixed_outcomes_exits_nonzero() {
    let temp = setup();
    temp.oj().args(&["run", "succeed"]).passes();
    temp.oj().args(&["run", "fail_cmd"]).passes();

    let done = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        let has_completed = out.contains("Completed");
        let has_failed = out.contains("failed");
        has_completed && has_failed
    });
    assert!(done, "pipelines should reach terminal states");

    let succeed_id = extract_pipeline_id(&temp, "succeed");
    let fail_id = extract_pipeline_id(&temp, "fail_cmd");

    // Wait --all with mixed outcomes should fail (exit non-zero)
    temp.oj()
        .args(&["pipeline", "wait", "--all", &succeed_id, &fail_id])
        .env("OJ_WAIT_POLL_MS", "10")
        .fails();
}
