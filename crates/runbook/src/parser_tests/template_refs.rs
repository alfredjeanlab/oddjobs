// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{parse_runbook, parse_runbook_with_format, Format, ParseError};

// ============================================================================
// Error: job-only namespaces rejected in command.run
// ============================================================================

#[test]
fn error_command_run_rejects_var_namespace() {
    let toml = r#"
[command.build]
args = "<name>"
run = "echo ${var.name}"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("var.name"),
        "error should mention the variable: {}",
        msg
    );
    assert!(
        msg.contains("args."),
        "error should suggest args namespace: {}",
        msg
    );
}

#[test]
fn error_command_run_rejects_input_namespace() {
    let toml = r#"
[command.build]
args = "<description>"
run = "echo ${input.description}"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("input.description"),
        "error should mention the variable: {}",
        msg
    );
    assert!(
        msg.contains("args."),
        "error should suggest args namespace: {}",
        msg
    );
}

#[test]
fn error_command_run_rejects_local_namespace() {
    let toml = r#"
[command.build]
args = "<name>"
run = "echo ${local.repo}"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("local.repo"),
        "error should mention the variable: {}",
        msg
    );
    assert!(
        msg.contains("job"),
        "error should mention job context: {}",
        msg
    );
}

#[test]
fn error_command_run_rejects_step_namespace() {
    let toml = r#"
[command.build]
args = "<name>"
run = "echo ${step.output}"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("step.output"),
        "error should mention the variable: {}",
        msg
    );
}

#[test]
fn error_command_run_rejects_dotted_workspace_namespace() {
    let toml = r#"
[command.build]
args = "<name>"
run = "echo ${workspace.root}"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("workspace.root"),
        "error should mention the variable: {}",
        msg
    );
    assert!(
        msg.contains("${workspace}"),
        "error should suggest plain workspace: {}",
        msg
    );
}

#[test]
fn error_hcl_command_run_rejects_var_namespace() {
    let hcl = r#"
command "build" {
  args = "<name>"
  run  = "echo ${var.name}"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("var.name"),
        "error should mention the variable: {}",
        msg
    );
}

// ============================================================================
// Valid: allowed namespaces in command.run
// ============================================================================

#[test]
fn parse_command_run_allows_args_namespace() {
    let toml = r#"
[command.build]
args = "<name>"
run = "echo ${args.name}"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.commands.contains_key("build"));
}

#[test]
fn parse_command_run_allows_plain_workspace() {
    let toml = r#"
[command.build]
args = "<name>"
run = "echo ${workspace}"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.commands.contains_key("build"));
}

#[test]
fn parse_command_run_allows_invoke_dir() {
    let toml = r#"
[command.build]
args = "<name>"
run = "echo ${invoke.dir}"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.commands.contains_key("build"));
}

#[test]
fn parse_command_run_allows_simple_vars() {
    let toml = r#"
[command.build]
args = "<name>"
run = "echo ${name} ${job_id}"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.commands.contains_key("build"));
}

#[test]
fn parse_command_job_directive_allows_var_namespace() {
    // ${var.*} in job steps is fine — it's the job's concern, not the command's
    let toml = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
vars = ["name"]
name = "${var.name}"

[[job.build.step]]
name = "init"
run = "echo ${var.name}"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.commands.contains_key("build"));
}
