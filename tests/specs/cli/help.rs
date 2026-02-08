//! CLI help output specs
//!
//! Verify help text displays for all commands.

use crate::prelude::*;

#[test]
fn oj_no_args_shows_usage_and_exits_zero() {
    cli().passes().stdout_has("Usage:");
}

#[test]
fn oj_help_shows_usage() {
    cli().args(&["--help"]).passes().stdout_has("Usage:");
}

#[test]
fn oj_run_help_shows_usage() {
    cli().args(&["run", "--help"]).passes().stdout_has("Usage:");
}

#[test]
fn oj_daemon_help_shows_subcommands() {
    cli()
        .args(&["daemon", "--help"])
        .passes()
        .stdout_has("start")
        .stdout_has("stop")
        .stdout_has("status");
}

#[test]
fn oj_job_help_shows_subcommands() {
    cli()
        .args(&["job", "--help"])
        .passes()
        .stdout_has("list")
        .stdout_has("show");
}

#[test]
fn oj_version_shows_version() {
    cli().args(&["--version"]).passes().stdout_has("0.1");
}

/// `oj runbook info` handles library files with const directives without crashing.
/// Regression test for bugs fixed in 5e5c0a9 and f0b3eaa where %{if}/%{endif}
/// directives caused parse failures in the info/search commands.
#[test]
fn runbook_info_handles_const_directives() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/libraries/constlib/main.hcl",
        r#"# Library with const directives

const "check" { default = "true" }

command "verify" {
  run = <<-SHELL
    %{ if const.check != "true" }
    ${raw(const.check)}
    %{ endif }
    echo "verified"
  SHELL
}
"#,
    );

    temp.oj()
        .args(&["runbook", "info", "constlib"])
        .passes()
        .stdout_has("constlib")
        .stdout_has("verify");
}

/// Imported commands show the library's description, not the importing file's.
/// Regression test for bug fixed in 98db6a4.
#[test]
fn help_for_imported_command_shows_library_description() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/libraries/doclib/main.hcl",
        r#"# Run automated tests
command "test" {
  run = "echo testing"
}
"#,
    );
    temp.file(
        ".oj/runbooks/base.hcl",
        r#"# Not the imported description
import "doclib" {}
"#,
    );

    // The listing should show the library's description for the imported command
    temp.oj()
        .args(&["run"])
        .passes()
        .stdout_has("test")
        .stdout_has("Run automated tests");
}
