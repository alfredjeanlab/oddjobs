// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Lexer tests for words containing assignment-like patterns.
//!
//! The lexer no longer recognizes assignment patterns - it emits Word tokens
//! for all `NAME=VALUE` patterns. Assignment detection is handled by the parser
//! at command-start position.

use crate::lexer::Lexer;
use crate::token::TokenKind;

// =============================================================================
// Basic Assignment-like Words
// =============================================================================

lex_tests! {
    simple_assignment: "FOO=bar" => [
        TokenKind::Word("FOO=bar".into())
    ],
    empty_value: "VAR=" => [
        TokenKind::Word("VAR=".into())
    ],
    underscore_name: "_VAR=value" => [
        TokenKind::Word("_VAR=value".into())
    ],
    numeric_suffix: "VAR1=test" => [
        TokenKind::Word("VAR1=test".into())
    ],
    lowercase_name: "foo=bar" => [
        TokenKind::Word("foo=bar".into())
    ],
    mixed_case: "FoO_BaR=value" => [
        TokenKind::Word("FoO_BaR=value".into())
    ],
    underscore_only: "_=val" => [
        TokenKind::Word("_=val".into())
    ],
    multiple_underscores: "__VAR__=x" => [
        TokenKind::Word("__VAR__=x".into())
    ],
}

// =============================================================================
// Value Edge Cases
// =============================================================================

lex_tests! {
    path_value: "PATH=/usr/bin:/bin" => [
        TokenKind::Word("PATH=/usr/bin:/bin".into())
    ],
    equals_in_value: "VAR=a=b=c" => [
        TokenKind::Word("VAR=a=b=c".into())
    ],
    numeric_value: "NUM=42" => [
        TokenKind::Word("NUM=42".into())
    ],
    special_chars_in_value: "URL=http://example.com" => [
        TokenKind::Word("URL=http://example.com".into())
    ],
    dots_in_value: "FILE=foo.bar.baz" => [
        TokenKind::Word("FILE=foo.bar.baz".into())
    ],
}

// =============================================================================
// Non-Assignments (Remain as Word - unchanged behavior)
// =============================================================================

lex_tests! {
    leading_digit: "123=foo" => [TokenKind::Word("123=foo".into())],
    no_name: "=value" => [TokenKind::Word("=value".into())],
    hyphen_in_name: "FOO-BAR=x" => [TokenKind::Word("FOO-BAR=x".into())],
    dot_in_name: "foo.bar=x" => [TokenKind::Word("foo.bar=x".into())],
    no_equals: "FOO" => [TokenKind::Word("FOO".into())],
    space_before_equals: "FOO =bar" => [
        TokenKind::Word("FOO".into()),
        TokenKind::Word("=bar".into()),
    ],
    space_after_equals: "FOO= bar" => [
        TokenKind::Word("FOO=".into()),
        TokenKind::Word("bar".into()),
    ],
}

// =============================================================================
// Multiple Assignment-like Words with Command
// =============================================================================

lex_tests! {
    two_assignments_cmd: "A=1 B=2 cmd" => [
        TokenKind::Word("A=1".into()),
        TokenKind::Word("B=2".into()),
        TokenKind::Word("cmd".into()),
    ],
    single_assignment_cmd: "FOO=bar echo hello" => [
        TokenKind::Word("FOO=bar".into()),
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
    ],
    assignment_cmd_args: "PATH=/bin ls -la" => [
        TokenKind::Word("PATH=/bin".into()),
        TokenKind::Word("ls".into()),
        TokenKind::Word("-la".into()),
    ],
    three_assignments: "A=1 B=2 C=3 cmd" => [
        TokenKind::Word("A=1".into()),
        TokenKind::Word("B=2".into()),
        TokenKind::Word("C=3".into()),
        TokenKind::Word("cmd".into()),
    ],
}

// =============================================================================
// Assignment-like Words with Operators
// =============================================================================

lex_tests! {
    assignment_with_pipe: "FOO=bar cmd | other" => [
        TokenKind::Word("FOO=bar".into()),
        TokenKind::Word("cmd".into()),
        TokenKind::Pipe,
        TokenKind::Word("other".into()),
    ],
    assignment_with_and: "FOO=bar cmd && next" => [
        TokenKind::Word("FOO=bar".into()),
        TokenKind::Word("cmd".into()),
        TokenKind::And,
        TokenKind::Word("next".into()),
    ],
    assignment_with_semicolon: "FOO=bar cmd ; other" => [
        TokenKind::Word("FOO=bar".into()),
        TokenKind::Word("cmd".into()),
        TokenKind::Semi,
        TokenKind::Word("other".into()),
    ],
}

// =============================================================================
// Quoted Values - Now Separate Tokens
// =============================================================================
//
// Since assignment detection moved to parser, quoted values after `=` become
// separate tokens. The parser handles concatenation at command-start position.

lex_tests! {
    double_quoted_value: r#"FOO="hello world""# => [
        TokenKind::Word("FOO=".into()),
        TokenKind::DoubleQuoted(vec![crate::ast::WordPart::double_quoted("hello world")])
    ],
    single_quoted_value: "FOO='hello world'" => [
        TokenKind::Word("FOO=".into()),
        TokenKind::SingleQuoted("hello world".into())
    ],
    double_quoted_cmd: r#"FOO="hello world" cmd"# => [
        TokenKind::Word("FOO=".into()),
        TokenKind::DoubleQuoted(vec![crate::ast::WordPart::double_quoted("hello world")]),
        TokenKind::Word("cmd".into()),
    ],
    single_quoted_cmd: "FOO='hello world' cmd" => [
        TokenKind::Word("FOO=".into()),
        TokenKind::SingleQuoted("hello world".into()),
        TokenKind::Word("cmd".into()),
    ],
    // Mixed quoted/unquoted: FOO=a"bc"d becomes Word("FOO=a"), DoubleQuoted("bc"), Word("d")
    mixed_quoted_unquoted: r#"FOO=a"bc"d"# => [
        TokenKind::Word("FOO=a".into()),
        TokenKind::DoubleQuoted(vec![crate::ast::WordPart::double_quoted("bc")]),
        TokenKind::Word("d".into())
    ],
    empty_double_quoted: r#"FOO="""# => [
        TokenKind::Word("FOO=".into()),
        TokenKind::DoubleQuoted(vec![])
    ],
    empty_single_quoted: "FOO=''" => [
        TokenKind::Word("FOO=".into()),
        TokenKind::SingleQuoted("".into())
    ],
}

// =============================================================================
// Span Accuracy Tests
// =============================================================================

span_tests! {
    assignment_span: "FOO=bar" => [(0, 7)],
    assignment_then_word: "A=1 cmd" => [(0, 3), (4, 7)],
    empty_value_span: "VAR=" => [(0, 4)],
    long_assignment_span: "LONG_VAR_NAME=some_value" => [(0, 24)],
    two_assignments_span: "A=1 B=2" => [(0, 3), (4, 7)],
}
