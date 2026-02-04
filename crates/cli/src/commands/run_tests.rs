// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::path::PathBuf;

use oj_runbook::{ArgSpec, CommandDef, RunDirective};

use super::{execute_shell_inline, format_available_commands};
use crate::color::HelpPrinter;

fn make_shell_command(name: &str, run: &str) -> CommandDef {
    CommandDef {
        name: name.to_string(),
        description: None,
        args: ArgSpec::default(),
        defaults: HashMap::new(),
        run: RunDirective::Shell(run.to_string()),
    }
}

fn make_shell_command_with_args(name: &str, args: &str, run: &str) -> CommandDef {
    CommandDef {
        name: name.to_string(),
        description: None,
        args: oj_runbook::parse_arg_spec(args).unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell(run.to_string()),
    }
}

#[test]
fn shell_inline_runs_command_successfully() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("marker.txt");

    let cmd_def = make_shell_command("test", &format!("touch {}", marker.display()));

    let result = execute_shell_inline(
        cmd_def.run.shell_command().unwrap(),
        &cmd_def,
        &[],
        &HashMap::new(),
        dir.path(),
        dir.path(),
        "test-ns",
    );

    assert!(result.is_ok());
    assert!(
        marker.exists(),
        "shell command should have created the marker file"
    );
}

#[test]
fn shell_inline_interpolates_args() {
    let dir = tempfile::tempdir().unwrap();
    let output_file = dir.path().join("output.txt");

    let cmd_def = make_shell_command_with_args(
        "greet",
        "<name>",
        &format!("echo ${{args.name}} > {}", output_file.display()),
    );

    let result = execute_shell_inline(
        cmd_def.run.shell_command().unwrap(),
        &cmd_def,
        &["world".to_string()],
        &HashMap::new(),
        dir.path(),
        dir.path(),
        "test-ns",
    );

    assert!(result.is_ok());
    let content = std::fs::read_to_string(&output_file).unwrap();
    assert_eq!(content.trim(), "world");
}

#[test]
fn shell_inline_interpolates_invoke_dir() {
    let dir = tempfile::tempdir().unwrap();
    let invoke = dir.path().join("subdir");
    std::fs::create_dir_all(&invoke).unwrap();
    let output_file = dir.path().join("invoke_dir.txt");

    let cmd_def = make_shell_command(
        "check-dir",
        &format!("echo ${{invoke.dir}} > {}", output_file.display()),
    );

    let result = execute_shell_inline(
        cmd_def.run.shell_command().unwrap(),
        &cmd_def,
        &[],
        &HashMap::new(),
        dir.path(),
        &invoke,
        "test-ns",
    );

    assert!(result.is_ok());
    let content = std::fs::read_to_string(&output_file).unwrap();
    assert_eq!(content.trim(), invoke.display().to_string());
}

#[test]
fn shell_inline_interpolates_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let output_file = dir.path().join("workspace.txt");

    let cmd_def = make_shell_command(
        "check-ws",
        &format!("echo ${{workspace}} > {}", output_file.display()),
    );

    let result = execute_shell_inline(
        cmd_def.run.shell_command().unwrap(),
        &cmd_def,
        &[],
        &HashMap::new(),
        dir.path(),
        dir.path(),
        "test-ns",
    );

    assert!(result.is_ok());
    let content = std::fs::read_to_string(&output_file).unwrap();
    assert_eq!(content.trim(), dir.path().display().to_string());
}

#[test]
fn shell_inline_does_not_inject_oj_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let output_file = dir.path().join("namespace.txt");

    // Ensure OJ_NAMESPACE is not inherited from this test process
    std::env::remove_var("OJ_NAMESPACE");

    let cmd_def = make_shell_command(
        "check-ns",
        &format!("echo ${{OJ_NAMESPACE:-unset}} > {}", output_file.display()),
    );

    let result = execute_shell_inline(
        cmd_def.run.shell_command().unwrap(),
        &cmd_def,
        &[],
        &HashMap::new(),
        dir.path(),
        dir.path(),
        "my-project",
    );

    assert!(result.is_ok());
    let content = std::fs::read_to_string(&output_file).unwrap();
    assert_eq!(
        content.trim(),
        "unset",
        "OJ_NAMESPACE should not be injected by shell-inline (only engine-managed workspaces)"
    );
}

#[test]
fn shell_inline_runs_in_project_root() {
    let dir = tempfile::tempdir().unwrap();
    let output_file = dir.path().join("cwd.txt");

    let cmd_def = make_shell_command("check-cwd", &format!("pwd > {}", output_file.display()));

    let result = execute_shell_inline(
        cmd_def.run.shell_command().unwrap(),
        &cmd_def,
        &[],
        &HashMap::new(),
        dir.path(),
        dir.path(),
        "test-ns",
    );

    assert!(result.is_ok());
    let content = std::fs::read_to_string(&output_file).unwrap();
    // Resolve symlinks for macOS /private/tmp
    let expected = std::fs::canonicalize(dir.path()).unwrap();
    let actual = PathBuf::from(content.trim());
    let actual_canon = std::fs::canonicalize(&actual).unwrap();
    assert_eq!(actual_canon, expected);
}

#[test]
fn shell_inline_leaves_unknown_vars_uninterpolated() {
    let dir = tempfile::tempdir().unwrap();
    let output_file = dir.path().join("unknown.txt");

    // ${pipeline_id} and ${name} should be left as-is
    let cmd_def = make_shell_command(
        "check-unknown",
        &format!("echo '${{pipeline_id}}' > {}", output_file.display()),
    );

    let result = execute_shell_inline(
        cmd_def.run.shell_command().unwrap(),
        &cmd_def,
        &[],
        &HashMap::new(),
        dir.path(),
        dir.path(),
        "test-ns",
    );

    assert!(result.is_ok());
    let content = std::fs::read_to_string(&output_file).unwrap();
    assert_eq!(content.trim(), "${pipeline_id}");
}

#[test]
fn shell_inline_with_named_args() {
    let dir = tempfile::tempdir().unwrap();
    let output_file = dir.path().join("named.txt");

    let cmd_def = make_shell_command_with_args(
        "greet",
        "<name> --greeting=hello",
        &format!(
            "echo ${{args.greeting}} ${{args.name}} > {}",
            output_file.display()
        ),
    );

    let mut named = HashMap::new();
    named.insert("greeting".to_string(), "hi".to_string());

    let result = execute_shell_inline(
        cmd_def.run.shell_command().unwrap(),
        &cmd_def,
        &["alice".to_string()],
        &named,
        dir.path(),
        dir.path(),
        "test-ns",
    );

    assert!(result.is_ok());
    let content = std::fs::read_to_string(&output_file).unwrap();
    assert_eq!(content.trim(), "hi alice");
}

#[test]
fn directive_is_shell_for_shell_commands() {
    let directive = RunDirective::Shell("echo hi".to_string());
    assert!(directive.is_shell());
    assert!(!directive.is_pipeline());
    assert!(!directive.is_agent());
}

#[test]
fn directive_is_pipeline_for_pipeline_commands() {
    let directive = RunDirective::Pipeline {
        pipeline: "build".to_string(),
    };
    assert!(!directive.is_shell());
    assert!(directive.is_pipeline());
}

#[test]
fn directive_is_agent_for_agent_commands() {
    let directive = RunDirective::Agent {
        agent: "planning".to_string(),
        attach: None,
    };
    assert!(!directive.is_shell());
    assert!(directive.is_agent());
}

#[test]
fn format_available_commands_empty_shows_no_commands() {
    let mut help = HelpPrinter::uncolored();
    format_available_commands(&mut help, &[], &[]);
    let buf = help.finish();

    assert!(buf.contains("No commands found."));
    assert!(buf.contains("Define commands in .oj/runbooks/*.hcl"));
    assert!(buf.contains("Usage: oj run <COMMAND>\n"));
    assert!(buf.contains("For more information, try 'oj run <COMMAND> -h'."));
    assert!(!buf.contains("Commands:"));
}

#[test]
fn format_available_commands_shows_commands() {
    let commands = vec![
        (
            "build".to_string(),
            make_shell_command("build", "make build"),
        ),
        (
            "greet".to_string(),
            make_shell_command_with_args("greet", "<name>", "echo ${args.name}"),
        ),
    ];

    let mut help = HelpPrinter::uncolored();
    format_available_commands(&mut help, &commands, &[]);
    let buf = help.finish();

    assert!(buf.contains("Commands:"));
    assert!(buf.contains("build"));
    assert!(buf.contains("greet"));
    assert!(!buf.contains("<name>")); // args no longer shown in listing
    assert!(!buf.contains("No commands found."));
}

#[test]
fn format_available_commands_shows_description() {
    let mut cmd = make_shell_command("deploy", "deploy.sh");
    cmd.description = Some("Deploy to production".to_string());
    let commands = vec![("deploy".to_string(), cmd)];

    let mut help = HelpPrinter::uncolored();
    format_available_commands(&mut help, &commands, &[]);
    let buf = help.finish();

    assert!(buf.contains("deploy"));
    assert!(buf.contains("Deploy to production"));
}

#[test]
fn format_available_commands_shows_warnings() {
    let commands = vec![(
        "build".to_string(),
        make_shell_command("build", "make build"),
    )];
    let warnings = vec!["runbooks/broken.hcl: expected `{`, found `}`".to_string()];

    let mut help = HelpPrinter::uncolored();
    format_available_commands(&mut help, &commands, &warnings);
    let buf = help.finish();

    assert!(buf.contains("Commands:"));
    assert!(buf.contains("build"));
    assert!(buf.contains("Warnings:"));
    assert!(buf.contains("runbooks/broken.hcl: expected `{`, found `}`"));
}

#[test]
fn format_available_commands_no_warnings_when_empty() {
    let commands = vec![(
        "build".to_string(),
        make_shell_command("build", "make build"),
    )];

    let mut help = HelpPrinter::uncolored();
    format_available_commands(&mut help, &commands, &[]);
    let buf = help.finish();

    assert!(!buf.contains("Warnings:"));
}

// Tests for --attach/--no-attach flag extraction from trailing args

/// Helper to simulate extracting attach override from raw args,
/// matching the logic in handle().
fn extract_attach_override(raw: &[&str]) -> (Option<bool>, Vec<String>) {
    let mut args: Vec<String> = raw.iter().map(|s| s.to_string()).collect();
    let mut cli_attach = None;
    args.retain(|a| {
        if a == "--attach" {
            cli_attach = Some(true);
            false
        } else if a == "--no-attach" {
            cli_attach = Some(false);
            false
        } else {
            true
        }
    });
    (cli_attach, args)
}

#[test]
fn attach_flag_extracted_from_trailing_args() {
    let (attach, remaining) = extract_attach_override(&["some-arg", "--attach"]);
    assert_eq!(attach, Some(true));
    assert_eq!(remaining, vec!["some-arg"]);
}

#[test]
fn no_attach_flag_extracted_from_trailing_args() {
    let (attach, remaining) = extract_attach_override(&["some-arg", "--no-attach"]);
    assert_eq!(attach, Some(false));
    assert_eq!(remaining, vec!["some-arg"]);
}

#[test]
fn no_flag_leaves_args_unchanged() {
    let (attach, remaining) = extract_attach_override(&["some-arg", "other"]);
    assert_eq!(attach, None);
    assert_eq!(remaining, vec!["some-arg", "other"]);
}

#[test]
fn attach_flag_between_positional_args() {
    let (attach, remaining) = extract_attach_override(&["first", "--attach", "second"]);
    assert_eq!(attach, Some(true));
    assert_eq!(remaining, vec!["first", "second"]);
}

#[test]
fn attach_override_precedence_cli_over_runbook() {
    // CLI --attach overrides runbook attach = false
    let cli_override = Some(true);
    let runbook_attach = Some(false);
    let should_attach = cli_override.or(runbook_attach).unwrap_or(false);
    assert!(should_attach);

    // CLI --no-attach overrides runbook attach = true
    let cli_override = Some(false);
    let runbook_attach = Some(true);
    let should_attach = cli_override.or(runbook_attach).unwrap_or(false);
    assert!(!should_attach);
}

#[test]
fn attach_override_precedence_runbook_over_default() {
    // No CLI flag, runbook says attach = true
    let cli_override: Option<bool> = None;
    let runbook_attach = Some(true);
    let should_attach = cli_override.or(runbook_attach).unwrap_or(false);
    assert!(should_attach);

    // No CLI flag, no runbook setting -> default false
    let cli_override: Option<bool> = None;
    let runbook_attach: Option<bool> = None;
    let should_attach = cli_override.or(runbook_attach).unwrap_or(false);
    assert!(!should_attach);
}
