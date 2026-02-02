//! CLI error handling specs
//!
//! Verify error messages for invalid commands and arguments.

use crate::prelude::*;

#[test]
fn run_unknown_command_shows_error() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    temp.oj()
        .args(&["run", "nonexistent", "arg"])
        .fails()
        .stderr_has("unknown command: nonexistent");
}

#[test]
fn run_missing_args_shows_error() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    // Runbook defines: args = "<name> <prompt>"
    // Running without args should error
    temp.oj()
        .args(&["run", "build"])
        .fails()
        .stderr_has("missing required argument: <name>");
}

#[test]
fn run_partial_args_shows_error() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    // Runbook defines: args = "<name> <prompt>"
    // Running with only first arg should error for second
    temp.oj()
        .args(&["run", "build", "myfeature"])
        .fails()
        .stderr_has("missing required argument: <prompt>");
}

#[test]
fn run_with_all_args_passes() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
    temp.oj().args(&["daemon", "start"]).passes();

    // Runbook defines: args = "<name> <prompt>"
    // Running with all required args should succeed
    temp.oj()
        .args(&["run", "build", "myfeature", "Add login button"])
        .passes()
        .stdout_has("Command build invoked");
}
