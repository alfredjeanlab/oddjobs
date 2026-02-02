// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Lexer tests for shell expansion constructs.
//!
//! These tests document how the lexer handles various shell expansion patterns
//! that may or may not be directly supported.

use crate::lexer::Lexer;
use crate::token::TokenKind;

// =============================================================================
// Process Substitution Tests
// =============================================================================
//
// Process substitution `<(cmd)` and `>(cmd)` is NOT directly supported.
// The lexer parses these as redirection followed by subshell tokens.
// This documents the current behavior.

#[test]
fn test_process_substitution_input_style() {
    // <(cmd) is parsed as: RedirectIn, LParen, Word, RParen
    let tokens = Lexer::tokenize("<(cmd)").unwrap();
    assert_eq!(tokens.len(), 4);
    assert!(matches!(tokens[0].kind, TokenKind::RedirectIn { fd: None }));
    assert_eq!(tokens[1].kind, TokenKind::LParen);
    assert_eq!(tokens[2].kind, TokenKind::Word("cmd".into()));
    assert_eq!(tokens[3].kind, TokenKind::RParen);
}

#[test]
fn test_process_substitution_output_style() {
    // >(cmd) is parsed as: RedirectOut, LParen, Word, RParen
    let tokens = Lexer::tokenize(">(cmd)").unwrap();
    assert_eq!(tokens.len(), 4);
    assert!(matches!(
        tokens[0].kind,
        TokenKind::RedirectOut { fd: None }
    ));
    assert_eq!(tokens[1].kind, TokenKind::LParen);
    assert_eq!(tokens[2].kind, TokenKind::Word("cmd".into()));
    assert_eq!(tokens[3].kind, TokenKind::RParen);
}

#[test]
fn test_process_substitution_in_command() {
    // diff <(cmd1) <(cmd2) - parsed as command with redirections + subshells
    let tokens = Lexer::tokenize("diff <(cmd1) <(cmd2)").unwrap();
    assert_eq!(tokens.len(), 9);
    assert_eq!(tokens[0].kind, TokenKind::Word("diff".into()));
    assert!(matches!(tokens[1].kind, TokenKind::RedirectIn { .. }));
    assert_eq!(tokens[2].kind, TokenKind::LParen);
    assert_eq!(tokens[3].kind, TokenKind::Word("cmd1".into()));
    assert_eq!(tokens[4].kind, TokenKind::RParen);
}

// =============================================================================
// Arithmetic Expansion Tests
// =============================================================================
//
// $((expr)) is treated as command substitution containing "(expr)".
// The lexer does NOT have special handling for arithmetic expansion.
// This is a design decision: inner parsing can distinguish $((...)) later.

#[test]
fn test_arithmetic_basic() {
    // $((1+2)) -> CommandSubstitution with content "(1+2)"
    let tokens = Lexer::tokenize("$((1+2))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "(1+2)".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_arithmetic_with_variables() {
    // $(($a + $b)) -> CommandSubstitution with content "($a + $b)"
    let tokens = Lexer::tokenize("$(($a + $b))").unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, "($a + $b)");
}

#[test]
fn test_arithmetic_complex() {
    // $((x * (y + z))) -> nested parens preserved
    let tokens = Lexer::tokenize("$((x * (y + z)))").unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, "(x * (y + z))");
}

#[test]
fn test_arithmetic_in_expression() {
    // echo $((2+3)) -> Word, CommandSubstitution
    let tokens = Lexer::tokenize("echo $((2+3))").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].kind, TokenKind::Word("echo".into()));
    let TokenKind::CommandSubstitution { content, .. } = &tokens[1].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, "(2+3)");
}

#[test]
fn test_arithmetic_vs_command_substitution() {
    // Distinguishing $(cmd) from $((math)):
    // - $(cmd) content doesn't start with (
    // - $((math)) content starts with (
    let cmd_tokens = Lexer::tokenize("$(echo)").unwrap();
    let math_tokens = Lexer::tokenize("$((1+1))").unwrap();

    let TokenKind::CommandSubstitution {
        content: cmd_content,
        ..
    } = &cmd_tokens[0].kind
    else {
        panic!("expected command substitution");
    };
    let TokenKind::CommandSubstitution {
        content: math_content,
        ..
    } = &math_tokens[0].kind
    else {
        panic!("expected command substitution");
    };

    // Downstream consumers can distinguish by checking if content starts with '('
    assert!(!cmd_content.starts_with('('));
    assert!(math_content.starts_with('('));
}
