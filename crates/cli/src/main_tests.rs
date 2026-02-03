// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use clap::error::ErrorKind;

use super::cli_command;

// -- Version flag -----------------------------------------------------------

#[test]
fn version_short_lowercase_v() {
    let err = cli_command()
        .try_get_matches_from(["oj", "-v"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayVersion);
}

#[test]
fn version_short_uppercase_v() {
    let err = cli_command()
        .try_get_matches_from(["oj", "-V"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayVersion);
}

#[test]
fn version_long() {
    let err = cli_command()
        .try_get_matches_from(["oj", "--version"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayVersion);
}

#[test]
fn version_uppercase_v_hidden_in_help() {
    let mut buf = Vec::new();
    cli_command().write_help(&mut buf).unwrap();
    let help = String::from_utf8(buf).unwrap();
    assert!(
        help.contains("-v, --version"),
        "help should show -v, --version"
    );
    assert!(
        !help.contains("-V,"),
        "help should not show -V as a visible flag"
    );
}

// -- Run subcommand help ----------------------------------------------------

#[test]
fn run_short_help_shows_custom_output() {
    let err = cli_command()
        .try_get_matches_from(["oj", "run", "-h"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    let help = err.to_string();
    assert!(
        help.contains("Usage: oj run <COMMAND> [ARGS]..."),
        "should show custom usage line, got:\n{help}"
    );
    // Must NOT contain clap's auto-generated argument descriptions
    assert!(
        !help.contains("Command to run"),
        "should not contain clap-generated arg help, got:\n{help}"
    );
}

#[test]
fn run_long_help_shows_custom_output() {
    let err = cli_command()
        .try_get_matches_from(["oj", "run", "--help"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    let help = err.to_string();
    assert!(
        help.contains("Usage: oj run <COMMAND> [ARGS]..."),
        "should show custom usage line, got:\n{help}"
    );
}

#[test]
fn help_subcommand_run_shows_custom_output() {
    let err = cli_command()
        .try_get_matches_from(["oj", "help", "run"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    let help = err.to_string();
    assert!(
        help.contains("Usage: oj run <COMMAND> [ARGS]..."),
        "should show custom usage line, got:\n{help}"
    );
    assert!(
        !help.contains("Command to run"),
        "should not contain clap-generated arg help, got:\n{help}"
    );
}

#[test]
fn run_help_and_help_run_are_identical() {
    let run_h = cli_command()
        .try_get_matches_from(["oj", "run", "-h"])
        .unwrap_err()
        .to_string();
    let help_run = cli_command()
        .try_get_matches_from(["oj", "help", "run"])
        .unwrap_err()
        .to_string();
    assert_eq!(run_h, help_run, "oj run -h and oj help run should match");
}
