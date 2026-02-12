// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Lexer tests for shell expansion constructs.
//!
//! These tests document how the lexer handles various shell expansion patterns
//! that may or may not be directly supported.

use crate::lexer::Lexer;
use crate::token::TokenKind;

//
// Process substitution `<(cmd)` and `>(cmd)` is NOT directly supported.
// The lexer parses these as redirection followed by subshell tokens.
// This documents the current behavior.

lex_tests! {
    process_substitution_input_style: "<(cmd)" => [
        TokenKind::RedirectIn { fd: None },
        TokenKind::LParen,
        TokenKind::Word("cmd".into()),
        TokenKind::RParen,
    ],
    process_substitution_output_style: ">(cmd)" => [
        TokenKind::RedirectOut { fd: None },
        TokenKind::LParen,
        TokenKind::Word("cmd".into()),
        TokenKind::RParen,
    ],
    process_substitution_in_command: "diff <(cmd1) <(cmd2)" => [
        TokenKind::Word("diff".into()),
        TokenKind::RedirectIn { fd: None },
        TokenKind::LParen,
        TokenKind::Word("cmd1".into()),
        TokenKind::RParen,
        TokenKind::RedirectIn { fd: None },
        TokenKind::LParen,
        TokenKind::Word("cmd2".into()),
        TokenKind::RParen,
    ],
}

//
// $((expr)) is treated as command substitution containing "(expr)".
// The lexer does NOT have special handling for arithmetic expansion.
// This is a design decision: inner parsing can distinguish $((...)) later.

lex_tests! {
    arithmetic_basic: "$((1+2))" => [
        TokenKind::CommandSubstitution {
            content: "(1+2)".into(),
            backtick: false,
        },
    ],
    arithmetic_with_variables: "$(($a + $b))" => [
        TokenKind::CommandSubstitution {
            content: "($a + $b)".into(),
            backtick: false,
        },
    ],
    arithmetic_complex: "$((x * (y + z)))" => [
        TokenKind::CommandSubstitution {
            content: "(x * (y + z))".into(),
            backtick: false,
        },
    ],
    arithmetic_in_expression: "echo $((2+3))" => [
        TokenKind::Word("echo".into()),
        TokenKind::CommandSubstitution {
            content: "(2+3)".into(),
            backtick: false,
        },
    ],
}

#[test]
fn test_arithmetic_vs_command_substitution() {
    // Distinguishing $(cmd) from $((math)):
    // - $(cmd) content doesn't start with (
    // - $((math)) content starts with (
    let cmd_tokens = Lexer::tokenize("$(echo)").unwrap();
    let math_tokens = Lexer::tokenize("$((1+1))").unwrap();

    let TokenKind::CommandSubstitution { content: cmd_content, .. } = &cmd_tokens[0].kind else {
        panic!("expected command substitution");
    };
    let TokenKind::CommandSubstitution { content: math_content, .. } = &math_tokens[0].kind else {
        panic!("expected command substitution");
    };

    // Downstream consumers can distinguish by checking if content starts with '('
    assert!(!cmd_content.starts_with('('));
    assert!(math_content.starts_with('('));
}
