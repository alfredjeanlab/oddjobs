// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Parser tests for nested structures: subshells, brace groups, and command substitution.

use super::helpers::{get_command, get_simple_command};
use crate::ast::*;
use crate::parser::{ParseError, Parser};

// =============================================================================
// Subshell Tests
// =============================================================================

#[test]
fn test_single_command_subshell() {
    let result = Parser::parse("(echo hello)").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::Subshell(subshell) => {
            assert_eq!(subshell.body.commands.len(), 1);
            let inner_cmd = get_simple_command(&subshell.body.commands[0]);
            assert_eq!(inner_cmd.args.len(), 1);
        }
        _ => panic!("expected subshell"),
    }
}

#[test]
fn test_subshell_multiple_commands() {
    let result = Parser::parse("(cmd1; cmd2)").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::Subshell(subshell) => {
            assert_eq!(subshell.body.commands.len(), 2);
        }
        _ => panic!("expected subshell"),
    }
}

#[test]
fn test_subshell_trailing_separator() {
    let result = Parser::parse("(cmd1; cmd2;)").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::Subshell(subshell) => {
            assert_eq!(subshell.body.commands.len(), 2);
        }
        _ => panic!("expected subshell"),
    }
}

#[test]
fn test_nested_subshells() {
    let result = Parser::parse("((nested))").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::Subshell(outer) => {
            assert_eq!(outer.body.commands.len(), 1);
            match get_command(&outer.body.commands[0]) {
                Command::Subshell(inner) => {
                    assert_eq!(inner.body.commands.len(), 1);
                }
                _ => panic!("expected nested subshell"),
            }
        }
        _ => panic!("expected subshell"),
    }
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

#[test]
fn test_empty_subshell() {
    let result = Parser::parse("()").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::Subshell(subshell) => {
            assert!(subshell.body.commands.is_empty());
        }
        _ => panic!("expected subshell"),
    }
}

// =============================================================================
// Brace Group Tests
// =============================================================================

#[test]
fn test_single_command_brace_group() {
    let result = Parser::parse("{ echo hello; }").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::BraceGroup(group) => {
            assert_eq!(group.body.commands.len(), 1);
        }
        _ => panic!("expected brace group"),
    }
}

#[test]
fn test_brace_group_multiple_commands() {
    let result = Parser::parse("{ cmd1; cmd2; }").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::BraceGroup(group) => {
            assert_eq!(group.body.commands.len(), 2);
        }
        _ => panic!("expected brace group"),
    }
}

#[test]
fn test_brace_group_no_trailing_semi() {
    // Some shells allow this, we should too
    let result = Parser::parse("{ cmd }").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::BraceGroup(group) => {
            assert_eq!(group.body.commands.len(), 1);
        }
        _ => panic!("expected brace group"),
    }
}

#[test]
fn test_nested_brace_groups() {
    let result = Parser::parse("{ { nested; }; }").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::BraceGroup(outer) => {
            assert_eq!(outer.body.commands.len(), 1);
            match get_command(&outer.body.commands[0]) {
                Command::BraceGroup(inner) => {
                    assert_eq!(inner.body.commands.len(), 1);
                }
                _ => panic!("expected nested brace group"),
            }
        }
        _ => panic!("expected brace group"),
    }
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

// =============================================================================
// Mixed Nesting Tests
// =============================================================================

#[test]
fn test_subshell_in_brace_group() {
    let result = Parser::parse("{ (subshell); }").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::BraceGroup(group) => {
            assert_eq!(group.body.commands.len(), 1);
            match get_command(&group.body.commands[0]) {
                Command::Subshell(_) => {}
                _ => panic!("expected subshell inside brace group"),
            }
        }
        _ => panic!("expected brace group"),
    }
}

#[test]
fn test_brace_group_in_subshell() {
    let result = Parser::parse("({ bracegroup; })").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::Subshell(subshell) => {
            assert_eq!(subshell.body.commands.len(), 1);
            match get_command(&subshell.body.commands[0]) {
                Command::BraceGroup(_) => {}
                _ => panic!("expected brace group inside subshell"),
            }
        }
        _ => panic!("expected subshell"),
    }
}

// =============================================================================
// Deep Nesting Tests
// =============================================================================

#[test]
fn test_deeply_nested_subshells() {
    // 5 levels deep
    let result = Parser::parse("(((((cmd)))))").unwrap();
    assert_eq!(result.commands.len(), 1);

    let mut current = get_command(&result.commands[0]);
    for _ in 0..5 {
        match current {
            Command::Subshell(subshell) => {
                assert_eq!(subshell.body.commands.len(), 1);
                current = get_command(&subshell.body.commands[0]);
            }
            Command::Simple(_) => break,
            _ => panic!("unexpected command type"),
        }
    }
}

#[test]
fn test_mixed_deep_nesting() {
    let result = Parser::parse("{ (cmd); }").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::BraceGroup(group) => {
            assert_eq!(group.body.commands.len(), 1);
            match get_command(&group.body.commands[0]) {
                Command::Subshell(subshell) => {
                    assert_eq!(subshell.body.commands.len(), 1);
                }
                _ => panic!("expected subshell"),
            }
        }
        _ => panic!("expected brace group"),
    }
}

// =============================================================================
// Span Tests
// =============================================================================

#[test]
fn test_subshell_span() {
    let result = Parser::parse("(cmd)").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::Subshell(subshell) => {
            assert_eq!(subshell.span.start, 0);
            assert_eq!(subshell.span.end, 5);
        }
        _ => panic!("expected subshell"),
    }
}

#[test]
fn test_brace_group_span() {
    let result = Parser::parse("{ cmd; }").unwrap();
    assert_eq!(result.commands.len(), 1);
    match get_command(&result.commands[0]) {
        Command::BraceGroup(group) => {
            assert_eq!(group.span.start, 0);
            assert_eq!(group.span.end, 8);
        }
        _ => panic!("expected brace group"),
    }
}

// =============================================================================
// Command Substitution Nesting Tests
// =============================================================================

#[test]
fn test_nested_command_substitution() {
    let result = Parser::parse("echo $(echo $(pwd))").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);

    // The argument is a command substitution
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(outer_body),
        ..
    } = &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // The outer substitution contains "echo $(pwd)"
    assert_eq!(outer_body.commands.len(), 1);
    let outer_inner_cmd = get_simple_command(&outer_body.commands[0]);
    assert_eq!(outer_inner_cmd.name.parts, vec![WordPart::literal("echo")]);

    // The inner substitution contains "pwd"
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(inner_body),
        ..
    } = &outer_inner_cmd.args[0].parts[0]
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
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(level1),
        ..
    } = &cmd.name.parts[0]
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
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(body),
        ..
    } = &cmd.args[0].parts[0]
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
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(body),
        ..
    } = &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // Substitution contains a subshell
    assert_eq!(body.commands.len(), 1);
    match get_command(&body.commands[0]) {
        Command::Subshell(subshell) => {
            assert_eq!(subshell.body.commands.len(), 1);
        }
        _ => panic!("expected subshell inside substitution"),
    }
}

#[test]
fn test_substitution_error_propagation() {
    // Error inside a substitution should be wrapped
    let result = Parser::parse("echo $(| bad)");
    assert!(matches!(result, Err(ParseError::InSubstitution { .. })));
}

// =============================================================================
// Extreme Nesting Edge Cases
// =============================================================================

#[test]
fn test_deeply_nested_mixed_constructs_10() {
    // 10 levels alternating subshell and brace group
    // ( { ( { ( { ( { ( { cmd; }; ); }; ); }; ); }; ); }
    let mut input = String::new();
    for i in 0..10 {
        if i % 2 == 0 {
            input.push('(');
        } else {
            input.push_str("{ ");
        }
    }
    input.push_str("cmd");
    for i in (0..10).rev() {
        if i % 2 == 0 {
            input.push(')');
        } else {
            input.push_str("; }");
        }
    }

    let result = Parser::parse(&input);
    assert!(
        result.is_ok(),
        "10-level mixed nesting should parse: {:?}",
        result.err()
    );
}

#[test]
fn test_pipeline_in_subshell() {
    // (a | b | c) - pipeline inside subshell
    let result = Parser::parse("(a | b | c)").unwrap();
    assert_eq!(result.commands.len(), 1);

    match get_command(&result.commands[0]) {
        Command::Subshell(subshell) => {
            assert_eq!(subshell.body.commands.len(), 1);
            match get_command(&subshell.body.commands[0]) {
                Command::Pipeline(p) => {
                    assert_eq!(p.commands.len(), 3);
                }
                _ => panic!("expected pipeline inside subshell"),
            }
        }
        _ => panic!("expected subshell"),
    }
}

#[test]
fn test_subshell_in_pipeline_limitation() {
    // (sub) | cmd - subshell as first element of pipeline
    // Currently NOT supported: after subshell, parser expects ';' or newline
    let result = Parser::parse("(sub) | cmd");
    assert!(result.is_err(), "subshell | cmd is currently not supported");
}

#[test]
fn test_brace_group_in_pipeline_limitation() {
    // { grp; } | cmd - brace group as first element of pipeline
    // Currently NOT supported: after brace group, parser expects ';' or newline
    let result = Parser::parse("{ grp; } | cmd");
    assert!(
        result.is_err(),
        "brace group | cmd is currently not supported"
    );
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
fn test_substitution_containing_pipeline() {
    // echo $(a | b) - command substitution with pipeline inside
    let result = Parser::parse("echo $(a | b)").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(body),
        ..
    } = &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // The inner content should be a pipeline
    assert_eq!(body.commands.len(), 1);
    match get_command(&body.commands[0]) {
        Command::Pipeline(p) => {
            assert_eq!(p.commands.len(), 2);
        }
        _ => panic!("expected pipeline in substitution"),
    }
}

#[test]
fn test_substitution_containing_logical_ops() {
    // echo $(a && b || c) - command substitution with logical operators
    let result = Parser::parse("echo $(a && b || c)").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(body),
        ..
    } = &cmd.args[0].parts[0]
    else {
        panic!("expected command substitution");
    };

    // The inner content should be an and-or list
    assert_eq!(body.commands.len(), 1);
    let and_or = &body.commands[0];
    assert_eq!(and_or.rest.len(), 2);
}

#[test]
fn test_empty_brace_group() {
    // { } - empty brace group (edge case)
    let result = Parser::parse("{ }").unwrap();
    assert_eq!(result.commands.len(), 1);

    match get_command(&result.commands[0]) {
        Command::BraceGroup(group) => {
            assert!(group.body.commands.is_empty());
        }
        _ => panic!("expected empty brace group"),
    }
}
