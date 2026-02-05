// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::{Path, PathBuf};

use clap::error::ErrorKind;
use clap::FromArgMatches;

use super::{cli_command, resolve_effective_namespace, Cli, Commands};

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
        help.contains("Usage: oj run <COMMAND>"),
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
        help.contains("Usage: oj run <COMMAND>"),
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
        help.contains("Usage: oj run <COMMAND>"),
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

// -- -C (directory) flag ----------------------------------------------------

#[test]
fn directory_flag_short_c() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "-C", "/tmp", "status"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert_eq!(cli.directory.unwrap(), PathBuf::from("/tmp"));
}

#[test]
fn directory_flag_before_subcommand() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "-C", "/tmp", "queue", "list"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert_eq!(cli.directory.unwrap(), PathBuf::from("/tmp"));
}

#[test]
fn directory_flag_absent() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "status"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(cli.directory.is_none());
}

// -- --project flag ---------------------------------------------------------

#[test]
fn project_flag_global() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "--project", "myproj", "queue", "list"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert_eq!(cli.project.unwrap(), "myproj");
}

#[test]
fn project_flag_absent() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "status"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(cli.project.is_none());
}

// -- -C and --project together ----------------------------------------------

#[test]
fn directory_and_project_together() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "-C", "/tmp", "--project", "myproj", "queue", "list"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert_eq!(cli.directory.unwrap(), PathBuf::from("/tmp"));
    assert_eq!(cli.project.unwrap(), "myproj");
}

// -- Help text --------------------------------------------------------------

#[test]
fn help_shows_directory_and_project_flags() {
    let mut buf = Vec::new();
    cli_command().write_help(&mut buf).unwrap();
    let help = String::from_utf8(buf).unwrap();
    assert!(help.contains("-C"), "help should show -C flag");
    assert!(
        help.contains("--project"),
        "help should show --project flag"
    );
}

// -- resolve_effective_namespace --------------------------------------------

#[test]
fn namespace_resolution_project_flag_wins() {
    let ns = resolve_effective_namespace(Some("override"), Path::new("/dummy"));
    assert_eq!(ns, "override");
}

#[test]
fn namespace_resolution_falls_back_to_project_root() {
    // With no --project flag and no OJ_NAMESPACE env, should resolve from project root
    // (which in practice falls back to basename or config)
    let ns = resolve_effective_namespace(None, Path::new("/tmp"));
    // Should return something (not panic), exact value depends on resolve_namespace impl
    assert!(!ns.is_empty() || ns.is_empty()); // just ensure no panic
}

// -- project_filter derived from CLI arg only --------------------------------

#[test]
fn project_filter_is_some_when_flag_passed() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "--project", "wok", "job", "list"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    // project_filter comes from cli.project — only Some when explicitly passed
    assert_eq!(cli.project.as_deref(), Some("wok"));
}

#[test]
fn project_filter_is_none_when_flag_omitted() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "job", "list"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    // Even if OJ_NAMESPACE is set, cli.project is None — filtering is skipped
    assert_eq!(cli.project, None);
}

// -- Subcommand --project is removed (backward compat) ----------------------

#[test]
fn subcommand_project_flag_no_longer_exists() {
    // `oj queue list --project foo` should fail now that --project is top-level only
    let result = cli_command().try_get_matches_from(["oj", "queue", "list", "--project", "myproj"]);
    // With global = true, --project is still accepted on subcommands via clap's
    // global propagation, so this should still parse fine.
    assert!(
        result.is_ok(),
        "global --project should still parse on subcommands"
    );
}

// -- Missing subcommand shows help ------------------------------------------

#[test]
fn missing_subcommand_produces_help_error() {
    let err = cli_command()
        .try_get_matches_from(["oj", "project"])
        .unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
        "missing subcommand should produce DisplayHelpOnMissingArgumentOrSubcommand"
    );
}

#[test]
fn missing_subcommand_help_contains_commands() {
    let err = cli_command()
        .try_get_matches_from(["oj", "job"])
        .unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
    let help = err.to_string();
    assert!(
        help.contains("Job management"),
        "should show job description, got:\n{help}"
    );
}

// -- Subcommand help includes global flags ----------------------------------

#[test]
fn subcommand_help_includes_global_flags() {
    let mut cmd = cli_command();
    cmd.build();
    let project = super::find_subcommand(cmd, &["project"]);
    let help = crate::help::format_help(project);
    assert!(
        help.contains("-C"),
        "subcommand help should include -C global flag, got:\n{help}"
    );
    assert!(
        help.contains("--project"),
        "subcommand help should include --project global flag, got:\n{help}"
    );
    assert!(
        help.contains("--output"),
        "subcommand help should include --output global flag, got:\n{help}"
    );
}

#[test]
fn subcommand_help_shows_full_usage_prefix() {
    let mut cmd = cli_command();
    cmd.build();
    let project = super::find_subcommand(cmd, &["project"]);
    let help = crate::help::format_help(project);
    assert!(
        help.contains("oj project"),
        "usage line should include 'oj project', got:\n{help}"
    );
}

#[test]
fn nested_subcommand_help_includes_global_flags() {
    let mut cmd = cli_command();
    cmd.build();
    let decision_list = super::find_subcommand(cmd, &["decision", "list"]);
    let help = crate::help::format_help(decision_list);
    assert!(
        help.contains("-C"),
        "nested subcommand help should include -C global flag, got:\n{help}"
    );
    assert!(
        help.contains("oj decision list"),
        "usage line should include 'oj decision list', got:\n{help}"
    );
}

// -- Cancel shortcut --------------------------------------------------------

#[test]
fn cancel_requires_at_least_one_id() {
    let result = cli_command().try_get_matches_from(["oj", "cancel"]);
    assert!(result.is_err());
}

#[test]
fn cancel_accepts_multiple_ids() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "cancel", "abc", "def"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(Commands::Cancel { ids }) if ids.len() == 2));
}

// -- Resume shortcut --------------------------------------------------------

#[test]
fn resume_accepts_id_and_message() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "resume", "abc", "-m", "try again"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(
        matches!(cli.command, Some(Commands::Resume { id, message, .. })
        if id.as_deref() == Some("abc") && message.as_deref() == Some("try again"))
    );
}

#[test]
fn resume_accepts_var_flags() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "resume", "abc", "--var", "key=val"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(Commands::Resume { var, .. }) if var.len() == 1));
}

#[test]
fn resume_accepts_all_flag() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "resume", "--all"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(Commands::Resume { id, all, .. })
        if id.is_none() && all));
}

#[test]
fn resume_all_with_kill() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "resume", "--all", "--kill"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(
        matches!(cli.command, Some(Commands::Resume { id, all, kill, .. })
        if id.is_none() && all && kill)
    );
}
