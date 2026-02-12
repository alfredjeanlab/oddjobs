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
    temp.oj().args(&["run", "hello"]).passes().stdout_has("hello-from-runbook");
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

    temp.oj().args(&["run", "greet", "world"]).passes().stdout_has("hello-world");
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
    temp.oj().args(&["daemon", "status"]).passes().stdout_has("not running");
}

/// Aliased import commands resolve correctly via `oj run alias:command`.
/// Regression test for bug fixed in 574f115 where aliased entity names
/// failed lookup because only exact HashMap matches were tried.
#[test]
fn aliased_import_command_resolves() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/libraries/mylib/main.hcl",
        r#"command "greet" {
  run = "echo hello-from-lib"
}
"#,
    );
    temp.file(
        ".oj/runbooks/test.hcl",
        r#"import "mylib" { alias = "x" }
"#,
    );

    temp.oj().args(&["run", "x:greet"]).passes().stdout_has("hello-from-lib");
}

/// Local entity with same base name as aliased import doesn't trigger
/// false "defined in multiple runbooks" error.
/// Regression test for bug fixed in 3866175.
#[test]
fn local_command_with_same_name_as_aliased_import() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/libraries/mylib/main.hcl",
        r#"command "greet" {
  run = "echo hello-from-lib"
}
"#,
    );
    temp.file(
        ".oj/runbooks/local.hcl",
        r#"command "greet" {
  run = "echo hello-from-local"
}
"#,
    );
    temp.file(
        ".oj/runbooks/imports.hcl",
        r#"import "mylib" { alias = "x" }
"#,
    );

    // Should resolve to local "greet" without duplicate definition error
    temp.oj().args(&["run", "greet"]).passes().stdout_has("hello-from-local");
}
