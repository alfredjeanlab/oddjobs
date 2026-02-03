// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::path::PathBuf;

use oj_runbook::{ArgSpec, CommandDef, RunDirective};

use super::execute_shell_inline;

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
fn shell_inline_sets_oj_namespace_env() {
    let dir = tempfile::tempdir().unwrap();
    let output_file = dir.path().join("namespace.txt");

    let cmd_def = make_shell_command(
        "check-ns",
        &format!("echo $OJ_NAMESPACE > {}", output_file.display()),
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
    assert_eq!(content.trim(), "my-project");
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
    };
    assert!(!directive.is_shell());
    assert!(directive.is_agent());
}
