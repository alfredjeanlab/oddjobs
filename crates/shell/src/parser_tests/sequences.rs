// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::helpers::get_simple_command;
use super::macros::{parse_span_tests, parse_tests};
use crate::ast::WordPart;
use crate::parser::Parser;

// =============================================================================
// Macro-based Tests
// =============================================================================

parse_tests! {
    macro_two_cmds: "a; b" => commands: 2,
    macro_three_cmds: "a; b; c" => commands: 3,
    macro_empty: "" => commands: 0,
    macro_whitespace: "   " => commands: 0,
}

parse_span_tests! {
    macro_span_single: "echo" => (0, 4),
    macro_span_two_words: "echo a" => (0, 6),
}

// =============================================================================
// Standard Tests
// =============================================================================

#[test]
fn test_semicolon_separated() {
    let result = Parser::parse("cmd1 ; cmd2").unwrap();
    assert_eq!(result.commands.len(), 2);

    let cmd1 = get_simple_command(&result.commands[0]);
    let cmd2 = get_simple_command(&result.commands[1]);

    assert_eq!(cmd1.name.parts, vec![WordPart::literal("cmd1")]);
    assert_eq!(cmd2.name.parts, vec![WordPart::literal("cmd2")]);
}

#[test]
fn test_newline_separated() {
    let result = Parser::parse("cmd1\ncmd2").unwrap();
    assert_eq!(result.commands.len(), 2);

    let cmd1 = get_simple_command(&result.commands[0]);
    let cmd2 = get_simple_command(&result.commands[1]);

    assert_eq!(cmd1.name.parts, vec![WordPart::literal("cmd1")]);
    assert_eq!(cmd2.name.parts, vec![WordPart::literal("cmd2")]);
}

#[test]
fn test_multiple_separators() {
    let result = Parser::parse("cmd1 ; cmd2 ; cmd3").unwrap();
    assert_eq!(result.commands.len(), 3);
}

#[test]
fn test_leading_separator() {
    let result = Parser::parse("; cmd").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("cmd")]);
}

#[test]
fn test_trailing_separator() {
    let result = Parser::parse("cmd ;").unwrap();
    assert_eq!(result.commands.len(), 1);
}

#[test]
fn test_multiple_leading_separators() {
    let result = Parser::parse(";;; cmd").unwrap();
    assert_eq!(result.commands.len(), 1);
}

#[test]
fn test_mixed_separators() {
    let result = Parser::parse("cmd1 ; cmd2\ncmd3").unwrap();
    assert_eq!(result.commands.len(), 3);
}

#[test]
fn test_complex_multiline_script() {
    let script = r#"
echo "starting"
ls -la
pwd
echo "done"
"#;
    let result = Parser::parse(script).unwrap();
    assert_eq!(result.commands.len(), 4);
}

#[test]
fn test_semicolons_with_args() {
    let result = Parser::parse("echo a ; echo b").unwrap();
    assert_eq!(result.commands.len(), 2);

    let cmd1 = get_simple_command(&result.commands[0]);
    let cmd2 = get_simple_command(&result.commands[1]);

    assert_eq!(cmd1.args.len(), 1);
    assert_eq!(cmd1.args[0].parts, vec![WordPart::literal("a")]);

    assert_eq!(cmd2.args.len(), 1);
    assert_eq!(cmd2.args[0].parts, vec![WordPart::literal("b")]);
}

#[test]
fn test_empty_between_separators() {
    // Multiple separators with nothing between should work
    let result = Parser::parse("cmd1 ;; cmd2").unwrap();
    assert_eq!(result.commands.len(), 2);
}

#[test]
fn test_only_separators() {
    let result = Parser::parse(";;;").unwrap();
    assert!(result.commands.is_empty());
}

#[test]
fn test_only_newlines() {
    let result = Parser::parse("\n\n\n").unwrap();
    assert!(result.commands.is_empty());
}

#[test]
fn test_command_list_span() {
    let result = Parser::parse("echo a ; echo b").unwrap();

    // The command list span should cover the entire input
    assert_eq!(result.span.start, 0);
    // 'echo a ; echo b' is 15 chars
    assert_eq!(result.span.end, 15);
}
