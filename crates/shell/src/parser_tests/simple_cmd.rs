// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::helpers::get_simple_command;
use super::macros::simple_cmd_tests;
use crate::ast::{SubstitutionBody, WordPart};
use crate::parser::Parser;

// =============================================================================
// Macro-based Tests
// =============================================================================

simple_cmd_tests! {
    macro_echo: "echo" => ("echo", 0),
    macro_echo_arg: "echo hello" => ("echo", 1),
    macro_ls: "ls -la /tmp" => ("ls", 2),
    macro_cat: "cat file1 file2 file3" => ("cat", 3),
}

// =============================================================================
// Standard Tests
// =============================================================================

#[test]
fn test_single_word_command() {
    let result = Parser::parse("echo").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("echo")]);
    assert!(cmd.args.is_empty());
}

#[test]
fn test_command_with_one_arg() {
    let result = Parser::parse("echo hello").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("echo")]);
    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts, vec![WordPart::literal("hello")]);
}

#[test]
fn test_command_with_multiple_args() {
    let result = Parser::parse("ls -la /tmp").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("ls")]);
    assert_eq!(cmd.args.len(), 2);
    assert_eq!(cmd.args[0].parts, vec![WordPart::literal("-la")]);
    assert_eq!(cmd.args[1].parts, vec![WordPart::literal("/tmp")]);
}

#[test]
fn test_command_with_variable() {
    let result = Parser::parse("echo $HOME").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("echo")]);
    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![WordPart::Variable {
            name: "HOME".into(),
            modifier: None
        }]
    );
}

#[test]
fn test_command_with_variable_modifier() {
    let result = Parser::parse("echo ${PATH:-/bin}").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![WordPart::Variable {
            name: "PATH".into(),
            modifier: Some(":-/bin".into())
        }]
    );
}

#[test]
fn test_command_with_command_substitution() {
    let result = Parser::parse("echo $(date)").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.args.len(), 1);

    // Verify the command substitution was parsed
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(body),
        backtick,
    } = &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };
    assert!(!backtick);
    assert_eq!(body.commands.len(), 1);

    // Verify inner command is "date"
    let inner_cmd = get_simple_command(&body.commands[0]);
    assert_eq!(inner_cmd.name.parts, vec![WordPart::literal("date")]);
}

#[test]
fn test_command_with_backtick_substitution() {
    let result = Parser::parse("echo `uname`").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.args.len(), 1);

    // Verify the backtick command substitution was parsed
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(body),
        backtick,
    } = &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };
    assert!(backtick);
    assert_eq!(body.commands.len(), 1);

    // Verify inner command is "uname"
    let inner_cmd = get_simple_command(&body.commands[0]);
    assert_eq!(inner_cmd.name.parts, vec![WordPart::literal("uname")]);
}

#[test]
fn test_command_span() {
    let result = Parser::parse("echo hello world").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    // Command spans from start of "echo" to end of "world"
    assert_eq!(cmd.span.start, 0);
    assert_eq!(cmd.span.end, 16);
}

#[test]
fn test_empty_input() {
    let result = Parser::parse("").unwrap();
    assert!(result.commands.is_empty());
}

#[test]
fn test_whitespace_only() {
    let result = Parser::parse("   \t  ").unwrap();
    assert!(result.commands.is_empty());
}

#[test]
fn test_variable_as_command() {
    let result = Parser::parse("$CMD arg1").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(
        cmd.name.parts,
        vec![WordPart::Variable {
            name: "CMD".into(),
            modifier: None
        }]
    );
    assert_eq!(cmd.args.len(), 1);
}

#[test]
fn test_command_substitution_as_command() {
    let result = Parser::parse("$(which python) --version").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);

    // Verify the command substitution was parsed as the command name
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(body),
        backtick,
    } = &cmd.name.parts[0]
    else {
        panic!("expected command substitution");
    };
    assert!(!backtick);
    assert_eq!(body.commands.len(), 1);

    // Verify inner command is "which python"
    let inner_cmd = get_simple_command(&body.commands[0]);
    assert_eq!(inner_cmd.name.parts, vec![WordPart::literal("which")]);
    assert_eq!(inner_cmd.args.len(), 1);
    assert_eq!(inner_cmd.args[0].parts, vec![WordPart::literal("python")]);

    assert_eq!(cmd.args.len(), 1);
}

// =============================================================================
// Backslash Escape Tests
// =============================================================================

#[test]
fn test_escaped_parens_in_find_command() {
    // find . \( -name foo \) should parse as a single simple command
    let result = Parser::parse("find . \\( -name foo \\)").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    super::helpers::assert_literal(&cmd.name, "find");
    assert_eq!(cmd.args.len(), 5);
    super::helpers::assert_literal(&cmd.args[0], ".");
    super::helpers::assert_literal(&cmd.args[1], "(");
    super::helpers::assert_literal(&cmd.args[2], "-name");
    super::helpers::assert_literal(&cmd.args[3], "foo");
    super::helpers::assert_literal(&cmd.args[4], ")");
}

#[test]
fn test_escaped_semicolon_is_argument() {
    // echo \; should parse as simple command with arg ";"
    let result = Parser::parse("echo \\;").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    super::helpers::assert_literal(&cmd.name, "echo");
    assert_eq!(cmd.args.len(), 1);
    super::helpers::assert_literal(&cmd.args[0], ";");
}

#[test]
fn test_escaped_pipe_is_argument() {
    // echo \| should parse as single command (not a job)
    let result = Parser::parse("echo \\|").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    super::helpers::assert_literal(&cmd.name, "echo");
    assert_eq!(cmd.args.len(), 1);
    super::helpers::assert_literal(&cmd.args[0], "|");
}

#[test]
fn test_escaped_backslash_is_literal() {
    // echo \\ should parse as simple command with arg "\"
    let result = Parser::parse("echo \\\\").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    super::helpers::assert_literal(&cmd.name, "echo");
    assert_eq!(cmd.args.len(), 1);
    super::helpers::assert_literal(&cmd.args[0], "\\");
}
