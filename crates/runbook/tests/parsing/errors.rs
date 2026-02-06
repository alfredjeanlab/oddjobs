// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Parse error tests and shell validation.

use oj_runbook::{parse_runbook, ParseError};

// ============================================================================
// Missing Required Fields
// ============================================================================

#[test]
fn missing_command_run() {
    super::assert_toml_err("[command.build]\nargs = \"<name>\"", &["run"]);
}

#[test]
fn missing_step_name() {
    let err = parse_runbook("[job.test]\n[[job.test.step]]\nrun = \"echo test\"").unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    super::assert_err_contains(&err, &["name"]);
}

#[test]
fn missing_step_run() {
    super::assert_toml_err("[job.test]\n[[job.test.step]]\nname = \"build\"", &["run"]);
}

// ============================================================================
// Invalid Shell Commands
// ============================================================================

#[test]
fn unterminated_quote_in_command_run() {
    let err = parse_runbook("[command.test]\nrun = \"echo 'unterminated\"").unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    super::assert_err_contains(&err, &["command.test.run"]);
}

#[test]
fn unterminated_subshell_in_step() {
    let toml = "[job.test]\n[[job.test.step]]\nname = \"broken\"\nrun = \"echo $(incomplete\"";
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    super::assert_err_contains(&err, &["job.test.step[0](broken).run"]);
}

#[test]
fn unterminated_quote_in_agent_run() {
    let err = parse_runbook("[agent.test]\nrun = \"claude \\\"unterminated\"").unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    super::assert_err_contains(&err, &["agent.test.run"]);
}

#[test]
fn error_context_includes_step_index() {
    let toml = r#"
[[job.deploy.step]]
name = "valid"
run = "echo ok"

[[job.deploy.step]]
name = "invalid"
run = "echo 'broken"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(
        matches!(err, ParseError::ShellError { ref location, .. } if location.contains("step[1]"))
    );
}

// ============================================================================
// Invalid TOML Structure
// ============================================================================

#[test]
fn command_not_table() {
    assert!(matches!(
        parse_runbook("[command]\nbuild = \"not a table\"").unwrap_err(),
        ParseError::Toml(_)
    ));
}

#[test]
fn invalid_run_directive() {
    assert!(matches!(
        parse_runbook("[command.test]\nrun = { invalid = \"key\" }").unwrap_err(),
        ParseError::Toml(_)
    ));
}

#[test]
fn job_not_table() {
    assert!(matches!(
        parse_runbook("[job]\nbuild = \"not a table\"").unwrap_err(),
        ParseError::Toml(_)
    ));
}

// ============================================================================
// Invalid Argument Specs
// ============================================================================

#[test]
fn duplicate_arg_name() {
    super::assert_toml_err(
        "[command.test]\nargs = \"<name> <name>\"\nrun = \"test.sh\"",
        &["duplicate"],
    );
}

#[test]
fn variadic_not_last() {
    super::assert_toml_err(
        "[command.test]\nargs = \"<files...> <extra>\"\nrun = \"test.sh\"",
        &["variadic"],
    );
}

#[test]
fn optional_before_required() {
    assert!(parse_runbook("[command.test]\nargs = \"[opt] <req>\"\nrun = \"test.sh\"").is_err());
}

#[test]
fn agent_missing_run() {
    super::assert_toml_err("[agent.test]\nprompt = \"Do something\"", &["run"]);
}

// ============================================================================
// Valid Shell Commands
// ============================================================================

#[yare::parameterized(
    simple              = { "cargo build --release" },
    pipes_and_logic     = { "cat file.txt | grep pattern | wc -l && echo success || echo failure" },
    subshell            = { "(cd /tmp && ls)" },
    brace_group         = { "{ echo hello; echo world; }" },
    variable_expansion  = { "echo ${VAR:-default} $HOME" },
    command_substitution = { "echo $(date +%Y-%m-%d)" },
    template_variables  = { "git worktree add worktrees/${name} -b feature/${name}" },
)]
fn valid_shell(run: &str) {
    let toml = format!("[command.c]\nrun = \"{run}\"");
    assert!(parse_runbook(&toml).is_ok(), "should parse: {run}");
}

#[test]
fn valid_shell_template_variables_in_job() {
    assert!(parse_runbook(
        "[[job.b.step]]\nname = \"init\"\nrun = \"echo ${message} | tee ${output_file}\""
    )
    .is_ok());
}

#[test]
fn unterminated_command_substitution() {
    let err = parse_runbook("[command.c]\nrun = \"echo $(date\"").unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
}

#[test]
fn directives_not_validated_as_shell() {
    let toml = r#"
[command.a]
run = { job = "build" }

[command.b]
run = { agent = "planner" }

[agent.planner]
run = "claude --print"

[job.build]
[[job.build.step]]
name = "run"
run = "cargo build"
"#;
    assert!(parse_runbook(toml).is_ok());
}
