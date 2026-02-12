// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Parser tests for nested structures: subshells, brace groups, and command substitution.

use super::helpers::{get_brace_group, get_command, get_simple_command, get_subshell};
use crate::ast::*;
use crate::parser::{ParseError, Parser};

#[yare::parameterized(
    single_cmd    = { "(echo hello)", 1 },
    trailing_semi = { "(cmd1; cmd2;)", 2 },
    empty         = { "()", 0 },
)]
fn subshell_command_count(input: &str, expected: usize) {
    let result = Parser::parse(input).unwrap();
    assert_eq!(result.commands.len(), 1);
    let subshell = get_subshell(&result.commands[0]);
    assert_eq!(subshell.body.commands.len(), expected);
}

#[test]
fn test_subshell_missing_rparen() {
    let result = Parser::parse("(cmd");
    assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
}

#[test]
fn test_unexpected_rparen() {
    let result = Parser::parse("cmd)");
    assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
}

#[yare::parameterized(
    single_cmd       = { "{ echo hello; }", 1 },
    multiple_cmds    = { "{ cmd1; cmd2; }", 2 },
    no_trailing_semi = { "{ cmd }", 1 },
    empty            = { "{ }", 0 },
)]
fn brace_group_command_count(input: &str, expected: usize) {
    let result = Parser::parse(input).unwrap();
    assert_eq!(result.commands.len(), 1);
    let group = get_brace_group(&result.commands[0]);
    assert_eq!(group.body.commands.len(), expected);
}

#[test]
fn test_brace_group_missing_rbrace() {
    let result = Parser::parse("{ cmd");
    assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
}

#[test]
fn test_unexpected_rbrace() {
    let result = Parser::parse("cmd }");
    assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
}

#[test]
fn test_subshell_in_brace_group() {
    let result = Parser::parse("{ (subshell); }").unwrap();
    assert_eq!(result.commands.len(), 1);
    let group = get_brace_group(&result.commands[0]);
    assert_eq!(group.body.commands.len(), 1);
    get_subshell(&group.body.commands[0]);
}

#[test]
fn test_brace_group_in_subshell() {
    let result = Parser::parse("({ bracegroup; })").unwrap();
    assert_eq!(result.commands.len(), 1);
    let subshell = get_subshell(&result.commands[0]);
    assert_eq!(subshell.body.commands.len(), 1);
    get_brace_group(&subshell.body.commands[0]);
}

#[test]
fn test_mixed_deep_nesting() {
    let result = Parser::parse("{ (cmd); }").unwrap();
    assert_eq!(result.commands.len(), 1);
    let group = get_brace_group(&result.commands[0]);
    assert_eq!(group.body.commands.len(), 1);
    let subshell = get_subshell(&group.body.commands[0]);
    assert_eq!(subshell.body.commands.len(), 1);
}

#[test]
fn test_subshell_span() {
    let result = Parser::parse("(cmd)").unwrap();
    assert_eq!(result.commands.len(), 1);
    let subshell = get_subshell(&result.commands[0]);
    assert_eq!(subshell.span.start, 0);
    assert_eq!(subshell.span.end, 5);
}

#[test]
fn test_brace_group_span() {
    let result = Parser::parse("{ cmd; }").unwrap();
    assert_eq!(result.commands.len(), 1);
    let group = get_brace_group(&result.commands[0]);
    assert_eq!(group.span.start, 0);
    assert_eq!(group.span.end, 8);
}

#[test]
fn test_nested_command_substitution() {
    let result = Parser::parse("echo $(echo $(pwd))").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);

    // The argument is a command substitution
    let WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(outer_body), .. } =
        &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // The outer substitution contains "echo $(pwd)"
    assert_eq!(outer_body.commands.len(), 1);
    let outer_inner_cmd = get_simple_command(&outer_body.commands[0]);
    assert_eq!(outer_inner_cmd.name.parts, vec![WordPart::literal("echo")]);

    // The inner substitution contains "pwd"
    let WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(inner_body), .. } =
        &outer_inner_cmd.args[0].parts[0]
    else {
        panic!("expected nested command substitution");
    };
    assert_eq!(inner_body.commands.len(), 1);
    let innermost_cmd = get_simple_command(&inner_body.commands[0]);
    assert_eq!(innermost_cmd.name.parts, vec![WordPart::literal("pwd")]);
}

#[test]
fn test_deeply_nested_substitutions() {
    // 3 levels deep
    let result = Parser::parse("$(a $(b $(c)))").unwrap();
    assert_eq!(result.commands.len(), 1);

    // Verify it parses without error - the structure is deeply nested
    let cmd = get_simple_command(&result.commands[0]);
    let WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(level1), .. } =
        &cmd.name.parts[0]
    else {
        panic!("expected command substitution at level 1");
    };
    assert_eq!(level1.commands.len(), 1);
}

#[test]
fn test_substitution_with_multiple_commands() {
    let result = Parser::parse("echo $(cmd1; cmd2)").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    let WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(body), .. } =
        &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // Substitution contains 2 commands
    assert_eq!(body.commands.len(), 2);
}

#[test]
fn test_substitution_with_subshell() {
    let result = Parser::parse("echo $((subshell))").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    let WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(body), .. } =
        &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // Substitution contains a subshell
    assert_eq!(body.commands.len(), 1);
    let subshell = get_subshell(&body.commands[0]);
    assert_eq!(subshell.body.commands.len(), 1);
}

#[test]
fn test_substitution_error_propagation() {
    // Error inside a substitution should be wrapped
    let result = Parser::parse("echo $(| bad)");
    assert!(matches!(result, Err(ParseError::InSubstitution { .. })));
}

#[test]
fn test_job_in_subshell() {
    // (a | b | c) - job inside subshell
    let result = Parser::parse("(a | b | c)").unwrap();
    assert_eq!(result.commands.len(), 1);
    let subshell = get_subshell(&result.commands[0]);
    assert_eq!(subshell.body.commands.len(), 1);
    match get_command(&subshell.body.commands[0]) {
        Command::Job(p) => assert_eq!(p.commands.len(), 3),
        _ => panic!("expected job inside subshell"),
    }
}

#[test]
fn test_subshell_in_job_limitation() {
    // (sub) | cmd - subshell as first element of job
    // Currently NOT supported: after subshell, parser expects ';' or newline
    let result = Parser::parse("(sub) | cmd");
    assert!(result.is_err(), "subshell | cmd is currently not supported");
}

#[test]
fn test_brace_group_in_job_limitation() {
    // { grp; } | cmd - brace group as first element of job
    // Currently NOT supported: after brace group, parser expects ';' or newline
    let result = Parser::parse("{ grp; } | cmd");
    assert!(result.is_err(), "brace group | cmd is currently not supported");
}

#[test]
fn test_logical_and_with_subshells() {
    // (a) && (b) - subshells connected by AND
    let result = Parser::parse("(a) && (b)").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 1);
    assert!(matches!(&and_or.first.command, Command::Subshell(_)));
    assert!(matches!(&and_or.rest[0].1.command, Command::Subshell(_)));
}

#[test]
fn test_substitution_containing_job() {
    // echo $(a | b) - command substitution with job inside
    let result = Parser::parse("echo $(a | b)").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    let WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(body), .. } =
        &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // The inner content should be a job
    assert_eq!(body.commands.len(), 1);
    match get_command(&body.commands[0]) {
        Command::Job(p) => {
            assert_eq!(p.commands.len(), 2);
        }
        _ => panic!("expected job in substitution"),
    }
}

#[test]
fn test_substitution_containing_logical_ops() {
    // echo $(a && b || c) - command substitution with logical operators
    let result = Parser::parse("echo $(a && b || c)").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    let WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(body), .. } =
        &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // The inner content should be an and-or list
    assert_eq!(body.commands.len(), 1);
    let and_or = &body.commands[0];
    assert_eq!(and_or.rest.len(), 2);
}
