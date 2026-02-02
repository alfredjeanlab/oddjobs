//! Pipeline show specs
//!
//! Verify pipeline show command behavior including prefix matching.

use crate::prelude::*;

#[test]
fn pipeline_list_empty() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["pipeline", "list"])
        .passes()
        .stdout_eq("No pipelines\n");
}

#[test]
fn pipeline_list_shows_running() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["run", "build", "test-feat", "do something"])
        .passes();

    // Wait for pipeline to appear (event processing is async)
    let found = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("test-feat")
    });
    assert!(found, "pipeline should appear in list");

    temp.oj()
        .args(&["pipeline", "list"])
        .passes()
        .stdout_has("test-feat")
        .stdout_has("build");
}

#[test]
fn pipeline_show_not_found() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["pipeline", "show", "nonexistent-id"])
        .passes()
        .stdout_eq("Pipeline not found: nonexistent-id\n");
}

#[test]
fn pipeline_show_by_prefix() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["run", "build", "prefix-test", "testing prefix"])
        .passes();

    // Wait for pipeline to appear (event processing is async)
    let found = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("prefix-test")
    });
    assert!(found, "pipeline should appear in list");

    // Get the truncated ID from list output
    let list_output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    let id_prefix = list_output
        .lines()
        .find(|l| l.contains("prefix-test"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find pipeline ID");

    // Show should work with the truncated ID
    temp.oj()
        .args(&["pipeline", "show", id_prefix])
        .passes()
        .stdout_has("Pipeline:")
        .stdout_has("prefix-test")
        .stdout_has("prompt: testing prefix");
}
