//! Job show specs
//!
//! Verify job show command behavior including prefix matching.

use crate::prelude::*;

#[test]
fn job_list_empty() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["job", "list"])
        .passes()
        .stdout_eq("No jobs\n");
}

#[test]
fn job_list_shows_running() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["run", "build", "test-feat", "do something"])
        .passes();

    // Wait for job to appear (event processing is async)
    let found = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("test-feat")
    });
    assert!(found, "job should appear in list");

    temp.oj()
        .args(&["job", "list"])
        .passes()
        .stdout_has("test-feat")
        .stdout_has("build");
}

#[test]
fn job_show_not_found() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["job", "show", "nonexistent-id"])
        .passes()
        .stdout_eq("Job not found: nonexistent-id\n");
}

#[test]
fn job_show_by_prefix() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["run", "build", "prefix-test", "testing prefix"])
        .passes();

    // Wait for job to appear (event processing is async)
    let found = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("prefix-test")
    });
    assert!(found, "job should appear in list");

    // Get the truncated ID from list output
    let list_output = temp.oj().args(&["job", "list"]).passes().stdout();
    let id_prefix = list_output
        .lines()
        .find(|l| l.contains("prefix-test"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find job ID");

    // Show should work with the truncated ID
    temp.oj()
        .args(&["job", "show", id_prefix])
        .passes()
        .stdout_has("Job:")
        .stdout_has("prefix-test")
        .stdout_has("prompt: testing prefix");
}
