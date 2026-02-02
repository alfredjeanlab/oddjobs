// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Additional parser tests for environment variable assignments.
//!
//! These tests complement `env_prefix.rs` with additional edge cases and
//! scenarios for assignment parsing.
//!
//! ## Current Behavior
//!
//! The parser recognizes `NAME=VALUE` patterns at command start and puts them
//! in `SimpleCommand.env`. After the first non-assignment word, all subsequent
//! `NAME=VALUE` patterns are treated as arguments.
//!
//! - `VAR=value cmd` → `env: [VAR=value], name: cmd, args: []`
//! - `cmd VAR=value` → `env: [], name: cmd, args: [VAR=value]`
//! - `VAR=value` (alone) → Standalone assignment (bash compatibility)

use super::helpers::{assert_literal, cmd_name, get_simple_command};
use crate::ast::WordPart;
use crate::parser::Parser;

// =============================================================================
// Standalone Assignment (Bash Compatibility)
// =============================================================================

#[test]
fn standalone_assignment() {
    // VAR=value with no command is allowed (bash compatibility)
    let ast = Parser::parse("VAR=value").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let cmd = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("value")]);
    // Command name is empty for standalone assignments
    assert!(cmd.name.parts.is_empty());
    assert!(cmd.args.is_empty());
}

#[test]
fn standalone_empty_assignment() {
    // VAR= with no command is allowed
    let ast = Parser::parse("VAR=").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let cmd = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("")]);
    assert!(cmd.name.parts.is_empty());
    assert!(cmd.args.is_empty());
}

// =============================================================================
// Assignment with Command
// =============================================================================

#[test]
fn assignment_with_command() {
    // VAR=value cmd → env contains VAR=value, name is cmd
    let ast = Parser::parse("VAR=value cmd").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let cmd = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("value")]);
    assert_literal(&cmd.name, "cmd");
    assert!(cmd.args.is_empty());
}

#[test]
fn multiple_assignments_with_command() {
    // A=1 B=2 C=3 cmd → env contains all assignments, name is cmd
    let ast = Parser::parse("A=1 B=2 C=3 cmd").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let cmd = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd.env.len(), 3);
    assert_eq!(cmd.env[0].name, "A");
    assert_eq!(cmd.env[1].name, "B");
    assert_eq!(cmd.env[2].name, "C");
    assert_literal(&cmd.name, "cmd");
}

// =============================================================================
// Assignment-like Pattern After Command (now works as argument)
// =============================================================================

#[test]
fn assignment_pattern_after_command_is_argument() {
    // `cmd VAR=value` passes `VAR=value` as an argument to cmd.
    // This is correct POSIX behavior - assignments are only recognized
    // at command-start position; elsewhere they're regular arguments.
    let ast = Parser::parse("cmd VAR=value").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let cmd = get_simple_command(&ast.commands[0]);
    assert!(cmd.env.is_empty()); // Not an env assignment
    assert_eq!(cmd_name(cmd), "cmd");
    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts, vec![WordPart::literal("VAR=value")]);
}

#[test]
fn flag_equals_value() {
    // ls --color=auto → --color=auto is an argument (Word, not Assignment)
    let ast = Parser::parse("ls --color=auto").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let cmd = get_simple_command(&ast.commands[0]);
    assert!(cmd.env.is_empty());
    assert_eq!(cmd_name(cmd), "ls");
    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts, vec![WordPart::literal("--color=auto")]);
}

// =============================================================================
// Assignment in Pipeline
// =============================================================================

#[test]
fn assignment_in_pipeline() {
    // VAR=value cmd | other → assignment applies to first command only
    let ast = Parser::parse("VAR=value cmd | other").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let pipeline = match &ast.commands[0].first.command {
        crate::ast::Command::Pipeline(p) => p,
        _ => panic!("Expected pipeline"),
    };

    assert_eq!(pipeline.commands.len(), 2);

    // First command has env assignment
    let first_cmd = &pipeline.commands[0];
    assert_eq!(first_cmd.env.len(), 1);
    assert_eq!(first_cmd.env[0].name, "VAR");
    assert_eq!(cmd_name(first_cmd), "cmd");

    // Second command has no assignment
    let second_cmd = &pipeline.commands[1];
    assert!(second_cmd.env.is_empty());
    assert_eq!(cmd_name(second_cmd), "other");
}

// =============================================================================
// Assignment with Variable Expansion
// =============================================================================

#[test]
fn assignment_with_variable_expansion() {
    // VAR=$OTHER cmd
    //
    // The lexer produces: Assignment { name: "VAR", value: "" }, Variable, Word
    // The parser now correctly concatenates adjacent tokens into the assignment value.
    let ast = Parser::parse("VAR=$OTHER cmd").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let cmd = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    // The assignment value is the variable
    assert_eq!(
        cmd.env[0].value.parts,
        vec![WordPart::Variable {
            name: "OTHER".into(),
            modifier: None,
        }]
    );
    // "cmd" is the command name
    assert_literal(&cmd.name, "cmd");
    // No arguments
    assert!(cmd.args.is_empty());
}

// =============================================================================
// Assignment in Sequences
// =============================================================================

#[test]
fn assignment_in_sequence() {
    // VAR=value cmd ; OTHER=x cmd2
    let ast = Parser::parse("VAR=value cmd ; OTHER=x cmd2").unwrap();
    assert_eq!(ast.commands.len(), 2);

    let cmd1 = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd1.env.len(), 1);
    assert_eq!(cmd1.env[0].name, "VAR");
    assert_eq!(cmd_name(cmd1), "cmd");

    let cmd2 = get_simple_command(&ast.commands[1]);
    assert_eq!(cmd2.env.len(), 1);
    assert_eq!(cmd2.env[0].name, "OTHER");
    assert_eq!(cmd_name(cmd2), "cmd2");
}

#[test]
fn assignment_with_and_or() {
    // VAR=1 cmd && OTHER=2 cmd2
    let ast = Parser::parse("VAR=1 cmd && OTHER=2 cmd2").unwrap();
    assert_eq!(ast.commands.len(), 1);

    let and_or = &ast.commands[0];
    assert_eq!(and_or.rest.len(), 1);

    // First command
    let first_cmd = match &and_or.first.command {
        crate::ast::Command::Simple(c) => c,
        _ => panic!("Expected simple command"),
    };
    assert_eq!(first_cmd.env.len(), 1);
    assert_eq!(first_cmd.env[0].name, "VAR");
    assert_eq!(cmd_name(first_cmd), "cmd");

    // Second command (after &&)
    let (_, second_item) = &and_or.rest[0];
    let second_cmd = match &second_item.command {
        crate::ast::Command::Simple(c) => c,
        _ => panic!("Expected simple command"),
    };
    assert_eq!(second_cmd.env.len(), 1);
    assert_eq!(second_cmd.env[0].name, "OTHER");
    assert_eq!(cmd_name(second_cmd), "cmd2");
}

// =============================================================================
// Span Verification
// =============================================================================

#[test]
fn assignment_span() {
    let ast = Parser::parse("VAR=value cmd").unwrap();
    let cmd = get_simple_command(&ast.commands[0]);
    // Command span covers from assignment start to command end
    assert_eq!(cmd.span.start, 0);
    assert_eq!(cmd.span.end, 13);
}

#[test]
fn assignment_with_command_span() {
    let ast = Parser::parse("VAR=value cmd arg").unwrap();
    let cmd = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd.span.start, 0);
    assert_eq!(cmd.span.end, 17);
}
