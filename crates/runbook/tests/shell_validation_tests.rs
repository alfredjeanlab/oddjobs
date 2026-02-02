// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Integration tests for shell command validation in runbook parsing.

use oj_runbook::{parse_runbook, ParseError};

#[test]
fn valid_shell_commands_parse_successfully() {
    let content = r#"
        [command.build]
        run = "cargo build --release"

        [[pipeline.deploy.step]]
        name = "push"
        run = "git push origin main"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn invalid_shell_in_command_returns_error() {
    let content = r#"
        [command.broken]
        run = "echo 'unterminated"
    "#;
    let err = parse_runbook(content).unwrap_err();
    assert!(
        matches!(err, ParseError::ShellError { ref location, .. } if location.contains("command.broken")),
        "Expected ShellError with location containing 'command.broken', got: {:?}",
        err
    );
}

#[test]
fn invalid_shell_in_pipeline_step_returns_error() {
    let content = r#"
        [[pipeline.build.step]]
        name = "init"
        run = "echo $(incomplete"
    "#;
    let err = parse_runbook(content).unwrap_err();
    assert!(
        matches!(err, ParseError::ShellError { ref location, .. } if location.contains("pipeline.build")),
        "Expected ShellError with location containing 'pipeline.build', got: {:?}",
        err
    );
}

#[test]
fn multiple_shell_commands_all_validated() {
    let content = r#"
        [command.first]
        run = "echo hello"

        [command.second]
        run = "echo 'unterminated"
    "#;
    let err = parse_runbook(content).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
}

#[test]
fn complex_valid_shell_commands() {
    let content = r#"
        [command.complex]
        run = "VAR=value cmd arg1 arg2 | grep pattern && echo done"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn shell_with_variable_expansion() {
    let content = r#"
        [command.vars]
        run = "echo ${VAR:-default} $HOME"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn shell_with_command_substitution() {
    let content = r#"
        [command.subst]
        run = "echo $(date +%Y-%m-%d)"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn pipeline_directive_not_validated_as_shell() {
    let content = r#"
        [command.delegate]
        run = { pipeline = "build" }

        [[pipeline.build.step]]
        name = "compile"
        run = "cargo build"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn agent_directive_not_validated_as_shell() {
    let content = r#"
        [command.ai]
        run = { agent = "planning" }

        [agent.planning]
        run = "claude --print"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn error_context_includes_step_index() {
    let content = r#"
        [[pipeline.deploy.step]]
        name = "valid"
        run = "echo ok"

        [[pipeline.deploy.step]]
        name = "invalid"
        run = "echo 'broken"
    "#;
    let err = parse_runbook(content).unwrap_err();
    assert!(
        matches!(err, ParseError::ShellError { ref location, .. } if location.contains("step[1]")),
        "Expected ShellError with location containing 'step[1]', got: {:?}",
        err
    );
}

#[test]
fn unterminated_command_substitution() {
    let content = r#"
        [command.broken]
        run = "echo $(date"
    "#;
    let err = parse_runbook(content).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
}

#[test]
fn shell_with_pipes_and_logical_operators() {
    let content = r#"
        [command.complex]
        run = "cat file.txt | grep pattern | wc -l && echo success || echo failure"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn shell_with_subshell() {
    let content = r#"
        [command.subshell]
        run = "(cd /tmp && ls)"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn shell_with_brace_group() {
    let content = r#"
        [command.braces]
        run = "{ echo hello; echo world; }"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn shell_with_template_variables() {
    let content = r#"
        [command.deploy]
        run = "git worktree add worktrees/${name} -b feature/${name}"
    "#;
    assert!(parse_runbook(content).is_ok());
}

#[test]
fn shell_with_template_variables_in_pipeline() {
    let content = r#"
        [[pipeline.build.step]]
        name = "init"
        run = "echo ${message} | tee ${output_file}"
    "#;
    assert!(parse_runbook(content).is_ok());
}
