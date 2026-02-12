// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Parser tests for environment variable prefix parsing.

use super::helpers::{assert_literal, cmd_name, get_job, get_simple_command};
use crate::ast::WordPart;
use crate::parser::Parser;
use crate::token::Span;

#[test]
fn test_single_env_prefix() {
    let result = Parser::parse("FOO=bar echo hello").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "FOO");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("bar")]);
    assert_literal(&cmd.name, "echo");
    assert_eq!(cmd.args.len(), 1);
    assert_literal(&cmd.args[0], "hello");
}

#[test]
fn test_multiple_env_prefixes() {
    let result = Parser::parse("A=1 B=2 C=3 cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 3);
    assert_eq!(cmd.env[0].name, "A");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("1")]);
    assert_eq!(cmd.env[1].name, "B");
    assert_eq!(cmd.env[1].value.parts, vec![WordPart::literal("2")]);
    assert_eq!(cmd.env[2].name, "C");
    assert_eq!(cmd.env[2].value.parts, vec![WordPart::literal("3")]);
    assert_literal(&cmd.name, "cmd");
}

#[test]
fn test_no_env_prefix() {
    let result = Parser::parse("echo hello").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert!(cmd.env.is_empty());
    assert_literal(&cmd.name, "echo");
    assert_eq!(cmd.args.len(), 1);
}

#[test]
fn test_empty_value_prefix() {
    let result = Parser::parse("VAR= cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("")]);
    assert_literal(&cmd.name, "cmd");
}

#[test]
fn test_path_value_prefix() {
    let result = Parser::parse("PATH=/usr/bin:/bin cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "PATH");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("/usr/bin:/bin")]);
}

#[test]
fn test_env_prefix_in_job() {
    let result = Parser::parse("FOO=bar cmd1 | cmd2").unwrap();
    let job = get_job(&result.commands[0]);

    // Prefix applies to first command only
    assert_eq!(job.commands.len(), 2);
    assert_eq!(job.commands[0].env.len(), 1);
    assert_eq!(job.commands[0].env[0].name, "FOO");
    assert_eq!(cmd_name(&job.commands[0]), "cmd1");

    // Second command has no prefix
    assert!(job.commands[1].env.is_empty());
    assert_eq!(cmd_name(&job.commands[1]), "cmd2");
}

#[test]
fn test_env_prefix_second_cmd_job() {
    let result = Parser::parse("cmd1 | FOO=bar cmd2").unwrap();
    let job = get_job(&result.commands[0]);

    // First command has no prefix
    assert!(job.commands[0].env.is_empty());
    assert_eq!(cmd_name(&job.commands[0]), "cmd1");

    // Second command has prefix
    assert_eq!(job.commands[1].env.len(), 1);
    assert_eq!(job.commands[1].env[0].name, "FOO");
    assert_eq!(cmd_name(&job.commands[1]), "cmd2");
}

#[test]
fn test_env_prefix_with_and() {
    let result = Parser::parse("FOO=bar cmd1 && cmd2").unwrap();
    let and_or = &result.commands[0];

    let first = match &and_or.first.command {
        crate::ast::Command::Simple(cmd) => cmd,
        _ => panic!("Expected simple command"),
    };
    assert_eq!(first.env.len(), 1);
    assert_eq!(first.env[0].name, "FOO");

    let (_, second) = &and_or.rest[0];
    let second_cmd = match &second.command {
        crate::ast::Command::Simple(cmd) => cmd,
        _ => panic!("Expected simple command"),
    };
    assert!(second_cmd.env.is_empty());
}

#[test]
fn test_env_prefix_with_semicolon() {
    let result = Parser::parse("FOO=bar cmd1; BAZ=qux cmd2").unwrap();

    let cmd1 = get_simple_command(&result.commands[0]);
    assert_eq!(cmd1.env.len(), 1);
    assert_eq!(cmd1.env[0].name, "FOO");
    assert_eq!(cmd_name(cmd1), "cmd1");

    let cmd2 = get_simple_command(&result.commands[1]);
    assert_eq!(cmd2.env.len(), 1);
    assert_eq!(cmd2.env[0].name, "BAZ");
    assert_eq!(cmd_name(cmd2), "cmd2");
}

#[test]
fn test_assignment_only() {
    // Standalone assignments are allowed (bash compatibility)
    let ast = Parser::parse("VAR=value").unwrap();
    assert_eq!(ast.commands.len(), 1);
    let cmd = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    assert!(cmd.name.parts.is_empty());
}

#[test]
fn test_multiple_assignment_only() {
    // Multiple standalone assignments are allowed
    let ast = Parser::parse("A=1 B=2").unwrap();
    assert_eq!(ast.commands.len(), 1);
    let cmd = get_simple_command(&ast.commands[0]);
    assert_eq!(cmd.env.len(), 2);
    assert_eq!(cmd.env[0].name, "A");
    assert_eq!(cmd.env[1].name, "B");
    assert!(cmd.name.parts.is_empty());
}

#[test]
fn test_env_prefix_assignment_span() {
    let result = Parser::parse("FOO=bar cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env[0].span, Span::new(0, 7));
}

#[test]
fn test_env_prefix_cmd_span() {
    let result = Parser::parse("FOO=bar cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    // Command span should cover from start of assignments to end of command
    assert_eq!(cmd.span, Span::new(0, 11));
}

#[test]
fn test_multiple_prefix_spans() {
    let result = Parser::parse("A=1 B=2 cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env[0].span, Span::new(0, 3));
    assert_eq!(cmd.env[1].span, Span::new(4, 7));
    assert_eq!(cmd.span, Span::new(0, 11));
}

#[test]
fn test_env_prefix_real_world_make() {
    let result = Parser::parse("CC=clang CFLAGS=-O2 make build").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 2);
    assert_eq!(cmd.env[0].name, "CC");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("clang")]);
    assert_eq!(cmd.env[1].name, "CFLAGS");
    assert_eq!(cmd.env[1].value.parts, vec![WordPart::literal("-O2")]);
    assert_literal(&cmd.name, "make");
    assert_eq!(cmd.args.len(), 1);
    assert_literal(&cmd.args[0], "build");
}

#[test]
fn test_env_prefix_real_world_node() {
    let result = Parser::parse("NODE_ENV=production npm start").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "NODE_ENV");
    assert_eq!(cmd.env[0].value.parts, vec![WordPart::literal("production")]);
    assert_literal(&cmd.name, "npm");
}

#[test]
fn test_env_prefix_real_world_debug() {
    let result = Parser::parse("DEBUG=1 RUST_BACKTRACE=full cargo test").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 2);
    assert_eq!(cmd.env[0].name, "DEBUG");
    assert_eq!(cmd.env[1].name, "RUST_BACKTRACE");
    assert_literal(&cmd.name, "cargo");
}

#[test]
fn test_env_prefix_value_with_variable() {
    // VAR=hello$world cmd → value should be Word([Lit("hello"), Var("world")])
    let result = Parser::parse("VAR=hello$SUFFIX cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    assert_eq!(
        cmd.env[0].value.parts,
        vec![
            WordPart::literal("hello"),
            WordPart::Variable { name: "SUFFIX".into(), modifier: None }
        ]
    );
    assert_literal(&cmd.name, "cmd");
}

#[test]
fn test_env_prefix_value_variable_only() {
    // VAR=$OTHER cmd → value should be Word([Var("OTHER")])
    let result = Parser::parse("VAR=$OTHER cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    assert_eq!(
        cmd.env[0].value.parts,
        vec![WordPart::Variable { name: "OTHER".into(), modifier: None }]
    );
}

#[test]
fn test_env_prefix_value_with_command_substitution() {
    // VAR=prefix$(date) cmd → value with concatenation
    let result = Parser::parse("VAR=prefix$(date) cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "VAR");
    assert_eq!(cmd.env[0].value.parts.len(), 2);
    assert_eq!(cmd.env[0].value.parts[0], WordPart::literal("prefix"));
    assert!(matches!(&cmd.env[0].value.parts[1], WordPart::CommandSubstitution { .. }));
}

#[test]
fn test_env_prefix_value_path_with_variable() {
    // PATH=$HOME/bin:/usr/bin cmd → value with multiple parts
    let result = Parser::parse("PATH=$HOME/bin:/usr/bin cmd").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.env.len(), 1);
    assert_eq!(cmd.env[0].name, "PATH");
    assert_eq!(
        cmd.env[0].value.parts,
        vec![
            WordPart::Variable { name: "HOME".into(), modifier: None },
            WordPart::literal("/bin:/usr/bin")
        ]
    );
}
