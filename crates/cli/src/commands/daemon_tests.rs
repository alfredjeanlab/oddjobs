// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

#![allow(clippy::unwrap_used)]

use clap::error::ErrorKind;
use clap::FromArgMatches;

// -- Version flag -----------------------------------------------------------

#[test]
fn daemon_version_short_v() {
    let matches = crate::cli_command()
        .try_get_matches_from(["oj", "daemon", "-v"])
        .unwrap();
    let cli = crate::Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(crate::Commands::Daemon(ref args)) if args.version));
}

#[test]
fn daemon_version_long() {
    let matches = crate::cli_command()
        .try_get_matches_from(["oj", "daemon", "--version"])
        .unwrap();
    let cli = crate::Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(crate::Commands::Daemon(ref args)) if args.version));
}

#[test]
fn daemon_version_hidden_uppercase_v() {
    let matches = crate::cli_command()
        .try_get_matches_from(["oj", "daemon", "-V"])
        .unwrap();
    let cli = crate::Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(crate::Commands::Daemon(ref args)) if args.version));
}

#[test]
fn daemon_version_hidden_in_help() {
    let mut cmd = crate::find_subcommand(crate::cli_command(), &["daemon"]);
    let mut buf = Vec::new();
    cmd.write_help(&mut buf).unwrap();
    let help = String::from_utf8(buf).unwrap();
    assert!(
        help.contains("-v, --version"),
        "daemon help should show -v, --version, got:\n{help}"
    );
    assert!(
        !help.contains("-V,"),
        "daemon help should not show -V as a visible flag, got:\n{help}"
    );
}

// -- No subcommand (colored help) -------------------------------------------

#[test]
fn daemon_no_subcommand_parses() {
    // `oj daemon` should parse successfully (subcommand is now optional)
    let matches = crate::cli_command()
        .try_get_matches_from(["oj", "daemon"])
        .unwrap();
    let cli = crate::Cli::from_arg_matches(&matches).unwrap();
    assert!(
        matches!(cli.command, Some(crate::Commands::Daemon(ref args)) if args.command.is_none() && !args.version)
    );
}

// -- Help colorization ------------------------------------------------------

#[test]
fn daemon_help_is_colorized() {
    let cmd = crate::find_subcommand(crate::cli_command(), &["daemon"]);
    let help = crate::help::format_help(cmd);
    // In test environment, should_colorize() may return false, so use
    // colorize_help directly to verify colorization works
    let colorized = crate::help::colorize_help(&help);
    // Section headers should be colored
    assert!(
        colorized.contains("\x1b[38;5;74m"),
        "daemon help should contain header color codes when colorized, got:\n{colorized}"
    );
}

#[test]
fn daemon_help_request_produces_display_help() {
    let err = crate::cli_command()
        .try_get_matches_from(["oj", "daemon", "-h"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayHelp);
}

#[test]
fn daemon_subcommand_help_contains_expected_content() {
    let cmd = crate::find_subcommand(crate::cli_command(), &["daemon"]);
    let help = crate::help::format_help(cmd);
    assert!(
        help.contains("Usage:"),
        "daemon help should contain Usage line, got:\n{help}"
    );
    assert!(
        help.contains("start"),
        "daemon help should mention start subcommand, got:\n{help}"
    );
    assert!(
        help.contains("stop"),
        "daemon help should mention stop subcommand, got:\n{help}"
    );
    assert!(
        help.contains("status"),
        "daemon help should mention status subcommand, got:\n{help}"
    );
}
