//! Tests for `oj run` command behavior.

use crate::prelude::*;

/// Shell commands execute inline without requiring a running daemon.
#[test]
fn shell_command_runs_without_daemon() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.hello]
run = "echo hello-from-runbook"
"#,
    );

    // Run without starting the daemon — should succeed for shell commands
    temp.oj()
        .args(&["run", "hello"])
        .passes()
        .stdout_has("hello-from-runbook");
}

/// Shell commands with arguments execute inline without a daemon.
#[test]
fn shell_command_with_args_runs_without_daemon() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.greet]
args = "<name>"
run = "echo hello-${args.name}"
"#,
    );

    temp.oj()
        .args(&["run", "greet", "world"])
        .passes()
        .stdout_has("hello-world");
}

/// Shell commands don't start the daemon.
#[test]
fn shell_command_does_not_start_daemon() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.hello]
run = "echo hello"
"#,
    );

    temp.oj().args(&["run", "hello"]).passes();

    // Daemon should not have been started
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("not running");
}
