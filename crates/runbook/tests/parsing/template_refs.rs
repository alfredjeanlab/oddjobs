// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Template variable namespace validation in command.run.

use oj_runbook::parse_runbook;

// ============================================================================
// Rejected Namespaces in command.run
// ============================================================================

#[test]
fn error_command_run_rejects_var_namespace() {
    super::assert_toml_err(
        "[command.build]\nargs = \"<name>\"\nrun = \"echo ${var.name}\"",
        &["var.name", "args."],
    );
}

#[test]
fn error_command_run_rejects_input_namespace() {
    super::assert_toml_err(
        "[command.build]\nargs = \"<description>\"\nrun = \"echo ${input.description}\"",
        &["input.description", "args."],
    );
}

#[test]
fn error_command_run_rejects_local_namespace() {
    super::assert_toml_err(
        "[command.build]\nargs = \"<name>\"\nrun = \"echo ${local.repo}\"",
        &["local.repo", "job"],
    );
}

#[test]
fn error_command_run_rejects_step_namespace() {
    super::assert_toml_err(
        "[command.build]\nargs = \"<name>\"\nrun = \"echo ${step.output}\"",
        &["step.output"],
    );
}

#[test]
fn error_command_run_rejects_dotted_workspace_namespace() {
    super::assert_toml_err(
        "[command.build]\nargs = \"<name>\"\nrun = \"echo ${workspace.root}\"",
        &["workspace.root", "${workspace}"],
    );
}

#[test]
fn hcl_command_run_rejects_var_namespace() {
    super::assert_hcl_err(
        "command \"build\" { args = \"<name>\"; run = \"echo ${var.name}\" }",
        &["var.name"],
    );
}

// ============================================================================
// Allowed Namespaces in command.run
// ============================================================================

#[yare::parameterized(
    args_namespace   = { "echo ${args.name}" },
    plain_workspace  = { "echo ${workspace}" },
    invoke_dir       = { "echo ${invoke.dir}" },
    simple_vars      = { "echo ${name} ${job_id}" },
)]
fn command_run_allows(run: &str) {
    let toml = format!("[command.build]\nargs = \"<name>\"\nrun = \"{run}\"");
    assert!(parse_runbook(&toml).is_ok(), "should allow: {run}");
}

#[test]
fn command_job_directive_allows_var_namespace() {
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
    assert!(parse_runbook(toml).is_ok());
}
