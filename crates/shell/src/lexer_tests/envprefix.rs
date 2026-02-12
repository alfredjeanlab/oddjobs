// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Environment prefix assignment lexer tests.
//!
//! Tests verifying how the lexer tokenizes `NAME=VALUE` patterns used for
//! environment variable prefix assignments in shell commands.
//!
//! ## Current Behavior
//!
//! The lexer emits Word tokens for `VAR=value` patterns. Assignment detection
//! is handled by the parser at command-start position.
//!
//! - `VAR=value` -> `Word("VAR=value")`
//! - `VAR=$OTHER` -> `Word("VAR=")`, `Variable("OTHER")`
//!   (the `$` triggers a new token)
//! - `VAR="quoted"` -> `Word("VAR=")`, `DoubleQuoted("quoted")`
//!   (the `"` triggers a new token)

use crate::lexer::Lexer;
use crate::token::TokenKind;

lex_tests! {
    assignment_single: "VAR=value" => [
        TokenKind::Word("VAR=value".into()),
    ],
    assignment_with_command: "VAR=value cmd" => [
        TokenKind::Word("VAR=value".into()),
        TokenKind::Word("cmd".into()),
    ],
    multiple_assignments: "A=1 B=2 C=3 cmd" => [
        TokenKind::Word("A=1".into()),
        TokenKind::Word("B=2".into()),
        TokenKind::Word("C=3".into()),
        TokenKind::Word("cmd".into()),
    ],
    empty_value: "VAR= cmd" => [
        TokenKind::Word("VAR=".into()),
        TokenKind::Word("cmd".into()),
    ],
    standalone_assignment: "VAR=value" => [
        TokenKind::Word("VAR=value".into()),
    ],
}

lex_tests! {
    equals_in_value: "VAR=a=b" => [
        TokenKind::Word("VAR=a=b".into()),
    ],
    special_chars_in_value: "VAR=a/b/c.txt" => [
        TokenKind::Word("VAR=a/b/c.txt".into()),
    ],
    // --color=auto is NOT an assignment (name doesn't match variable pattern)
    assignment_vs_flag: "--color=auto" => [TokenKind::Word("--color=auto".into())],
    underscore_in_name: "_VAR=value" => [
        TokenKind::Word("_VAR=value".into()),
    ],
    number_after_first_char: "VAR2=value" => [
        TokenKind::Word("VAR2=value".into()),
    ],
    path_value: "PATH=/usr/bin:/bin" => [
        TokenKind::Word("PATH=/usr/bin:/bin".into()),
    ],
    url_value: "URL=https://example.com/path" => [
        TokenKind::Word("URL=https://example.com/path".into()),
    ],
    equals_only: "VAR=" => [
        TokenKind::Word("VAR=".into()),
    ],
    multiple_equals: "A=1=2=3" => [
        TokenKind::Word("A=1=2=3".into()),
    ],
}

lex_tests! {
    // When `$` is encountered, it triggers variable tokenization, so the
    // word ends at `=` and the variable becomes a separate token.
    variable_expansion_in_assignment: "VAR=$OTHER" => [
        TokenKind::Word("VAR=".into()),
        TokenKind::Variable { name: "OTHER".into(), modifier: None },
    ],
    braced_variable_in_assignment: "VAR=${OTHER}" => [
        TokenKind::Word("VAR=".into()),
        TokenKind::Variable { name: "OTHER".into(), modifier: None },
    ],
    variable_with_default_in_assignment: "VAR=${OTHER:-default}" => [
        TokenKind::Word("VAR=".into()),
        TokenKind::Variable { name: "OTHER".into(), modifier: Some(":-default".into()) },
    ],
}

span_tests! {
    assignment_span: "VAR=value" => [(0, 9)],
    assignment_with_command_span: "VAR=value cmd" => [(0, 9), (10, 13)],
    empty_value_span: "VAR= cmd" => [(0, 4), (5, 8)],
    variable_in_assignment_span: "VAR=$OTHER" => [(0, 4), (4, 10)],
}

// The lexer tokenizes words regardless of position.
// Position-based semantics (e.g., "assignment only at start") are the parser's
// responsibility.

lex_tests! {
    // After a word, VAR=value is still tokenized as Word
    // (parser decides if it's an argument vs env prefix)
    assignment_after_command: "cmd VAR=value" => [
        TokenKind::Word("cmd".into()),
        TokenKind::Word("VAR=value".into()),
    ],
    assignment_in_job: "VAR=value cmd | other" => [
        TokenKind::Word("VAR=value".into()),
        TokenKind::Word("cmd".into()),
        TokenKind::Pipe,
        TokenKind::Word("other".into()),
    ],
}

#[test]
fn quoted_assignment_value() {
    // Quoted values are now separate tokens; parser concatenates them
    let tokens = Lexer::tokenize(r#"VAR="value with spaces" cmd"#).unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].kind, TokenKind::Word("VAR=".into()));
    assert!(matches!(tokens[1].kind, TokenKind::DoubleQuoted(_)));
    assert_eq!(tokens[2].kind, TokenKind::Word("cmd".into()));
}

#[test]
fn single_quoted_assignment_value() {
    let tokens = Lexer::tokenize("VAR='single quoted' cmd").unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].kind, TokenKind::Word("VAR=".into()));
    assert_eq!(tokens[1].kind, TokenKind::SingleQuoted("single quoted".into()));
    assert_eq!(tokens[2].kind, TokenKind::Word("cmd".into()));
}
