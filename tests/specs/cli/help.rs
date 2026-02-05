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
