// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Quote lexer tests: single quotes, double quotes, escape sequences,
//! error cases, and integration with other tokens.

use crate::ast::{SubstitutionBody, WordPart};
use crate::lexer::{Lexer, LexerError};
use crate::token::TokenKind;

/// Helper to create a DoubleQuoted token with a single literal part.
fn dq(s: &str) -> TokenKind {
    TokenKind::DoubleQuoted(vec![WordPart::double_quoted(s)])
}

// =============================================================================
// Single Quote Tests
// =============================================================================

lex_tests! {
    // Basic single quotes
    single_quote_basic: "'hello'" => [TokenKind::SingleQuoted("hello".into())],
    single_quote_with_spaces: "'hello world'" => [TokenKind::SingleQuoted("hello world".into())],

    // Empty single quotes
    single_quote_empty: "''" => [TokenKind::SingleQuoted("".into())],

    // Single quotes with special characters preserved (literal)
    single_quote_preserves_dollar: "'$VAR'" => [TokenKind::SingleQuoted("$VAR".into())],
    single_quote_preserves_braced_var: "'${HOME}'" => [TokenKind::SingleQuoted("${HOME}".into())],
    single_quote_preserves_backslash_n: r"'line\nbreak'" => [TokenKind::SingleQuoted(r"line\nbreak".into())],
    single_quote_preserves_backslash_t: r"'tab\there'" => [TokenKind::SingleQuoted(r"tab\there".into())],
    single_quote_preserves_backslash: r"'back\\slash'" => [TokenKind::SingleQuoted(r"back\\slash".into())],
    single_quote_preserves_double_quote: r#"'has "double" quotes'"# => [TokenKind::SingleQuoted(r#"has "double" quotes"#.into())],
    single_quote_preserves_backtick: "'`cmd`'" => [TokenKind::SingleQuoted("`cmd`".into())],
    single_quote_preserves_operators: "'a && b | c'" => [TokenKind::SingleQuoted("a && b | c".into())],

    // Single quotes adjacent to words
    single_quote_after_word: "cmd'arg'" => [
        TokenKind::Word("cmd".into()),
        TokenKind::SingleQuoted("arg".into()),
    ],
    word_after_single_quote: "'arg'cmd" => [
        TokenKind::SingleQuoted("arg".into()),
        TokenKind::Word("cmd".into()),
    ],
    single_quote_between_words: "a'b'c" => [
        TokenKind::Word("a".into()),
        TokenKind::SingleQuoted("b".into()),
        TokenKind::Word("c".into()),
    ],

    // Multiple consecutive single quoted strings
    multiple_single_quotes: "'a''b''c'" => [
        TokenKind::SingleQuoted("a".into()),
        TokenKind::SingleQuoted("b".into()),
        TokenKind::SingleQuoted("c".into()),
    ],

    // Unicode in single quotes
    single_quote_unicode: "'hello 世界'" => [TokenKind::SingleQuoted("hello 世界".into())],
    single_quote_emoji: "'🦀 rust'" => [TokenKind::SingleQuoted("🦀 rust".into())],

    // Single quotes in command context
    echo_single_quoted: "echo 'hello'" => [
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("hello".into()),
    ],
    echo_multiple_single_quoted_args: "echo 'a' 'b'" => [
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("a".into()),
        TokenKind::SingleQuoted("b".into()),
    ],

    // Newlines in single quotes (multi-line)
    single_quote_with_newline: "'line1\nline2'" => [TokenKind::SingleQuoted("line1\nline2".into())],
    single_quote_with_crlf: "'line1\r\nline2'" => [TokenKind::SingleQuoted("line1\r\nline2".into())],
}

// =============================================================================
// Double Quote Tests
// =============================================================================

lex_tests! {
    // Basic double quotes
    double_quote_basic: r#""hello""# => [dq("hello")],
    double_quote_with_spaces: r#""hello world""# => [dq("hello world")],

    // Empty double quotes
    double_quote_empty: r#""""# => [TokenKind::DoubleQuoted(vec![])],

    // Escape sequences
    double_quote_escape_backslash: r#""back\\slash""# => [dq("back\\slash")],
    double_quote_escape_newline: r#""line\nbreak""# => [dq("line\nbreak")],
    double_quote_escape_tab: r#""tab\there""# => [dq("tab\there")],
    double_quote_escape_double: r#""quote\"here""# => [dq("quote\"here")],
    double_quote_escape_single: r#""apos\'trophe""# => [dq("apos'trophe")],

    // Multiple escape sequences
    double_quote_multiple_escapes: r#""a\nb\tc\\d""# => [dq("a\nb\tc\\d")],
    double_quote_adjacent_escapes: r#""\n\t\\\"\'""# => [dq("\n\t\\\"'")],

    // Double quotes adjacent to words
    double_quote_after_word: r#"cmd"arg""# => [
        TokenKind::Word("cmd".into()),
        dq("arg"),
    ],
    word_after_double_quote: r#""arg"cmd"# => [
        dq("arg"),
        TokenKind::Word("cmd".into()),
    ],
    double_quote_between_words: r#"a"b"c"# => [
        TokenKind::Word("a".into()),
        dq("b"),
        TokenKind::Word("c".into()),
    ],

    // Multiple consecutive double quoted strings
    multiple_double_quotes: r#""a""b""c""# => [
        dq("a"),
        dq("b"),
        dq("c"),
    ],

    // Unicode in double quotes
    double_quote_unicode: r#""hello 世界""# => [dq("hello 世界")],
    double_quote_emoji: r#""🦀 rust""# => [dq("🦀 rust")],

    // Double quotes in command context
    echo_double_quoted: r#"echo "hello""# => [
        TokenKind::Word("echo".into()),
        dq("hello"),
    ],
    echo_multiple_double_quoted_args: r#"echo "a" "b""# => [
        TokenKind::Word("echo".into()),
        dq("a"),
        dq("b"),
    ],

    // Newlines in double quotes (multi-line)
    double_quote_with_literal_newline: "\"line1\nline2\"" => [dq("line1\nline2")],
    double_quote_with_crlf: "\"line1\r\nline2\"" => [dq("line1\r\nline2")],
}

// =============================================================================
// Variables in Double Quotes (Parsed into Separate Parts)
// =============================================================================

#[test]
fn double_quote_simple_variable() {
    // Boundary literals are emitted for word splitting support
    let tokens = Lexer::tokenize(r#""$VAR""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted(""),
            WordPart::Variable {
                name: "VAR".into(),
                modifier: None,
            },
            WordPart::double_quoted(""),
        ])
    );
}

#[test]
fn double_quote_braced_variable() {
    // Boundary literals are emitted for word splitting support
    let tokens = Lexer::tokenize(r#""${HOME}""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted(""),
            WordPart::Variable {
                name: "HOME".into(),
                modifier: None,
            },
            WordPart::double_quoted(""),
        ])
    );
}

#[test]
fn double_quote_variable_with_modifier() {
    // Boundary literals are emitted for word splitting support
    let tokens = Lexer::tokenize(r#""${HOME:-/tmp}""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted(""),
            WordPart::Variable {
                name: "HOME".into(),
                modifier: Some(":-/tmp".into()),
            },
            WordPart::double_quoted(""),
        ])
    );
}

#[test]
fn double_quote_variable_in_path() {
    // Leading boundary literal emitted for word splitting support
    let tokens = Lexer::tokenize(r#""$HOME/bin""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted(""),
            WordPart::Variable {
                name: "HOME".into(),
                modifier: None,
            },
            WordPart::double_quoted("/bin"),
        ])
    );
}

#[test]
fn double_quote_mixed_text_and_variable() {
    let tokens = Lexer::tokenize(r#""hello $USER, welcome""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted("hello "),
            WordPart::Variable {
                name: "USER".into(),
                modifier: None,
            },
            WordPart::double_quoted(", welcome"),
        ])
    );
}

#[test]
fn double_quote_multiple_variables() {
    // Boundary literals are emitted for word splitting support
    let tokens = Lexer::tokenize(r#""$HOME/.config/$APP""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted(""),
            WordPart::Variable {
                name: "HOME".into(),
                modifier: None,
            },
            WordPart::double_quoted("/.config/"),
            WordPart::Variable {
                name: "APP".into(),
                modifier: None,
            },
            WordPart::double_quoted(""),
        ])
    );
}

#[test]
fn double_quote_command_substitution() {
    // Trailing boundary literal emitted for word splitting support
    let tokens = Lexer::tokenize(r#""today is $(date)""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted("today is "),
            WordPart::CommandSubstitution {
                body: SubstitutionBody::Unparsed("date".into()),
                backtick: false,
            },
            WordPart::double_quoted(""),
        ])
    );
}

#[test]
fn double_quote_backtick_substitution() {
    // Trailing boundary literal emitted for word splitting support
    let tokens = Lexer::tokenize(r#""today is `date`""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted("today is "),
            WordPart::CommandSubstitution {
                body: SubstitutionBody::Unparsed("date".into()),
                backtick: true,
            },
            WordPart::double_quoted(""),
        ])
    );
}

#[test]
fn double_quote_escaped_dollar() {
    // \$ should produce a literal $, not a variable
    let tokens = Lexer::tokenize(r#""price is \$10""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![WordPart::double_quoted("price is $10")])
    );
}

#[test]
fn double_quote_dollar_at_end() {
    // $ followed by non-variable char should become literal $
    // Note: trailing boundary literal emitted after the $ literal
    let tokens = Lexer::tokenize(r#""cost: $""#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::DoubleQuoted(vec![
            WordPart::double_quoted("cost: "),
            WordPart::double_quoted("$"),
            WordPart::double_quoted(""),
        ])
    );
}

// =============================================================================
// Mixed Quote Tests
// =============================================================================

lex_tests! {
    // Adjacent quotes of different types
    single_then_double: r#"'a'"b""# => [
        TokenKind::SingleQuoted("a".into()),
        dq("b"),
    ],
    double_then_single: r#""a"'b'"# => [
        dq("a"),
        TokenKind::SingleQuoted("b".into()),
    ],
    alternating_quotes: r#"'a'"b"'c'"# => [
        TokenKind::SingleQuoted("a".into()),
        dq("b"),
        TokenKind::SingleQuoted("c".into()),
    ],

    // Mixed quotes with words
    word_single_double: r#"cmd'a'"b""# => [
        TokenKind::Word("cmd".into()),
        TokenKind::SingleQuoted("a".into()),
        dq("b"),
    ],

    // Mixed quotes in command
    echo_mixed_quotes: r#"echo 'single' "double""# => [
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("single".into()),
        dq("double"),
    ],
}

// =============================================================================
// Integration Tests (Quotes with other tokens)
// =============================================================================

lex_tests! {
    // Quotes with operators
    quotes_with_pipe: r#"echo 'a' | cat"# => [
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("a".into()),
        TokenKind::Pipe,
        TokenKind::Word("cat".into()),
    ],
    quotes_with_and: r#"echo "ok" && echo 'done'"# => [
        TokenKind::Word("echo".into()),
        dq("ok"),
        TokenKind::And,
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("done".into()),
    ],
    quotes_with_semicolon: r#"echo 'a'; echo "b""# => [
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("a".into()),
        TokenKind::Semi,
        TokenKind::Word("echo".into()),
        dq("b"),
    ],

    // Quotes with redirections
    quotes_with_redirect_out: r#"echo "hello" > file"# => [
        TokenKind::Word("echo".into()),
        dq("hello"),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("file".into()),
    ],
    quotes_with_redirect_in: r#"cat < 'input'"# => [
        TokenKind::Word("cat".into()),
        TokenKind::RedirectIn { fd: None },
        TokenKind::SingleQuoted("input".into()),
    ],

    // Quotes at end of input
    single_quote_at_eof: "echo ''" => [
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("".into()),
    ],
    double_quote_at_eof: r#"echo """# => [
        TokenKind::Word("echo".into()),
        TokenKind::DoubleQuoted(vec![]),
    ],
}

// =============================================================================
// Span Tests
// =============================================================================

span_tests! {
    span_single_quote: "'hello'" => [(0, 7)],
    span_double_quote: r#""hello""# => [(0, 7)],
    span_empty_single: "''" => [(0, 2)],
    span_empty_double: r#""""# => [(0, 2)],
    span_word_then_single: "cmd'arg'" => [(0, 3), (3, 8)],
    span_word_then_double: r#"cmd"arg""# => [(0, 3), (3, 8)],
    span_two_single_quotes: "'a' 'b'" => [(0, 3), (4, 7)],
    span_adjacent_quotes: "'a''b'" => [(0, 3), (3, 6)],
}

// =============================================================================
// Error Cases
// =============================================================================

lex_error_tests! {
    // Unterminated single quote
    unterminated_single_immediate: "'" => LexerError::UnterminatedSingleQuote { .. },
    unterminated_single_with_content: "'hello" => LexerError::UnterminatedSingleQuote { .. },
    unterminated_single_after_word: "cmd '" => LexerError::UnterminatedSingleQuote { .. },
    unterminated_single_multiline: "'a\nb" => LexerError::UnterminatedSingleQuote { .. },

    // Unterminated double quote
    unterminated_double_immediate: "\"" => LexerError::UnterminatedDoubleQuote { .. },
    unterminated_double_with_content: "\"hello" => LexerError::UnterminatedDoubleQuote { .. },
    unterminated_double_after_word: "cmd \"" => LexerError::UnterminatedDoubleQuote { .. },
    unterminated_double_multiline: "\"a\nb" => LexerError::UnterminatedDoubleQuote { .. },

    // Invalid escape sequence in double quotes
    invalid_escape_a: r#""\a""# => LexerError::InvalidEscape { ch: 'a', .. },
    invalid_escape_x: r#""\x""# => LexerError::InvalidEscape { ch: 'x', .. },
    invalid_escape_0: r#""\0""# => LexerError::InvalidEscape { ch: '0', .. },
    invalid_escape_r: r#""\r""# => LexerError::InvalidEscape { ch: 'r', .. },

    // Trailing backslash in double quotes
    trailing_backslash_in_double: "\"hello\\" => LexerError::TrailingBackslash { .. },
    trailing_backslash_alone_in_double: "\"\\" => LexerError::TrailingBackslash { .. },
}

// =============================================================================
// Error Span Accuracy Tests
// =============================================================================

#[test]
fn error_span_unterminated_single_quote() {
    let result = Lexer::tokenize("'hello");
    let err = result.unwrap_err();
    match err {
        LexerError::UnterminatedSingleQuote { span } => {
            assert_eq!(span.start, 0, "span should start at opening quote");
            assert_eq!(span.end, 6, "span should end at end of content");
        }
        other => panic!("expected UnterminatedSingleQuote, got {:?}", other),
    }
}

#[test]
fn error_span_unterminated_double_quote() {
    let result = Lexer::tokenize("\"hello");
    let err = result.unwrap_err();
    match err {
        LexerError::UnterminatedDoubleQuote { span } => {
            assert_eq!(span.start, 0, "span should start at opening quote");
            assert_eq!(span.end, 6, "span should end at end of content");
        }
        other => panic!("expected UnterminatedDoubleQuote, got {:?}", other),
    }
}

#[test]
fn error_span_invalid_escape() {
    let result = Lexer::tokenize(r#""ab\xcd""#);
    let err = result.unwrap_err();
    match err {
        LexerError::InvalidEscape { ch, span } => {
            assert_eq!(ch, 'x', "should capture the invalid escape char");
            assert_eq!(span.start, 3, "span should start at backslash");
            assert_eq!(span.end, 5, "span should end after escape char");
        }
        other => panic!("expected InvalidEscape, got {:?}", other),
    }
}

#[test]
fn error_span_trailing_backslash() {
    let result = Lexer::tokenize("\"test\\");
    let err = result.unwrap_err();
    match err {
        LexerError::TrailingBackslash { span } => {
            assert_eq!(span.start, 5, "span should start at backslash");
            assert_eq!(span.end, 6, "span should end after backslash");
        }
        other => panic!("expected TrailingBackslash, got {:?}", other),
    }
}

#[test]
fn error_span_unterminated_after_word() {
    let result = Lexer::tokenize("cmd 'arg");
    let err = result.unwrap_err();
    match err {
        LexerError::UnterminatedSingleQuote { span } => {
            assert_eq!(span.start, 4, "span should start at opening quote");
            assert_eq!(span.end, 8, "span should end at end of content");
        }
        other => panic!("expected UnterminatedSingleQuote, got {:?}", other),
    }
}

// =============================================================================
// Command Substitution Quote Tests
// =============================================================================

/// Test cases for quote handling inside command substitutions.
/// Format: (input, expected_content)
const QUOTE_CASES: &[(&str, &str)] = &[
    // Basic quotes
    (r#"$(echo "hello")"#, r#"echo "hello""#),
    ("$(echo 'hello')", "echo 'hello'"),
    // Adjacent quotes
    (r#"$(echo 'a'"b")"#, r#"echo 'a'"b""#),
    (r#"$(echo "a"'b')"#, r#"echo "a"'b'"#),
    (r#"$('a'"b"'c')"#, r#"'a'"b"'c'"#),
    // Empty quotes
    (r#"$(echo "")"#, r#"echo """#),
    ("$(echo '')", "echo ''"),
    (r#"$(echo ""'')"#, r#"echo ""''"#),
    // Quotes at boundaries
    (r#"$("x")"#, r#""x""#),
    ("$('x')", "'x'"),
    // Nested quote characters (quote char inside opposite quote)
    (r#"$(echo "it's")"#, r#"echo "it's""#),
    (r#"$(echo 'say "hi"')"#, r#"echo 'say "hi"'"#),
    // Unicode in quotes
    (r#"$(echo "你好")"#, r#"echo "你好""#),
    ("$(echo '🦦')", "echo '🦦'"),
    // Real-world patterns
    (
        r#"$(git log --format="%H %s")"#,
        r#"git log --format="%H %s""#,
    ),
    ("$(find . -name '*.rs')", "find . -name '*.rs'"),
    (
        r#"$(curl -H "Authorization: Bearer $TOKEN")"#,
        r#"curl -H "Authorization: Bearer $TOKEN""#,
    ),
    (r#"$(echo "$HOME"/.config)"#, r#"echo "$HOME"/.config"#),
    // Complex escaping
    (r#"$(echo "\"")"#, r#"echo "\"""#),
    (r#"$(echo "\\n")"#, r#"echo "\\n""#),
];

#[test]
fn test_quote_cases() {
    for (input, expected) in QUOTE_CASES {
        let result = Lexer::tokenize(input);
        match result {
            Ok(tokens) => {
                assert_eq!(tokens.len(), 1, "input: {}", input);
                let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
                    panic!("expected CommandSubstitution for input: {}", input);
                };
                assert_eq!(content, *expected, "input: {}", input);
            }
            Err(e) => panic!("failed to tokenize {:?}: {}", input, e),
        }
    }
}

/// Test cases for quotes containing parentheses (critical for depth tracking).
const PAREN_IN_QUOTE_CASES: &[(&str, &str)] = &[
    (r#"$(echo ")")"#, r#"echo ")""#),
    (r#"$(echo "(")"#, r#"echo "(""#),
    ("$(echo ')')", "echo ')'"),
    ("$(echo '(')", "echo '('"),
    (r#"$(echo "()")"#, r#"echo "()""#),
    (r#"$(echo "(())")"#, r#"echo "(())""#),
    (r#"$(test "(a)" && (b))"#, r#"test "(a)" && (b)"#),
];

#[test]
fn test_paren_in_quote_cases() {
    for (input, expected) in PAREN_IN_QUOTE_CASES {
        let result = Lexer::tokenize(input);
        match result {
            Ok(tokens) => {
                assert_eq!(tokens.len(), 1, "input: {}", input);
                let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
                    panic!("expected CommandSubstitution for input: {}", input);
                };
                assert_eq!(content, *expected, "input: {}", input);
            }
            Err(e) => panic!("failed to tokenize {:?}: {}", input, e),
        }
    }
}

// =============================================================================
// Command Substitution Integration Tests
// =============================================================================

#[test]
fn test_quote_adjacent_to_operator() {
    // Quotes should not interfere with operator recognition outside $(...)
    // Note: This tests the main lexer, not just command substitution
    let tokens = Lexer::tokenize("echo hello|grep hi").unwrap();
    assert_eq!(tokens.len(), 5);
    assert_eq!(tokens[2].kind, TokenKind::Pipe);
}

#[test]
fn test_subst_with_quoted_operator() {
    // Quoted operators inside substitution
    let tokens = Lexer::tokenize(r#"$(echo "a|b")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: r#"echo "a|b""#.into(),
            backtick: false,
        }
    );
}

#[test]
fn test_subst_adjacent_to_redirection() {
    // $(cmd) > file
    let tokens = Lexer::tokenize("$(echo hi) > out.txt").unwrap();
    assert_eq!(tokens.len(), 3);
    assert!(matches!(
        tokens[0].kind,
        TokenKind::CommandSubstitution { .. }
    ));
    assert!(matches!(tokens[1].kind, TokenKind::RedirectOut { .. }));
}

#[test]
fn test_variable_in_quoted_subst() {
    // Verify $VAR inside quoted substitution is preserved
    let tokens = Lexer::tokenize(r#"$(echo "$HOME")"#).unwrap();
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: r#"echo "$HOME""#.into(),
            backtick: false,
        }
    );
}

#[test]
fn test_subst_before_operator() {
    let tokens = Lexer::tokenize("$(a) && $(b)").unwrap();
    assert_eq!(tokens.len(), 3);
    assert!(matches!(
        tokens[0].kind,
        TokenKind::CommandSubstitution { .. }
    ));
    assert_eq!(tokens[1].kind, TokenKind::And);
    assert!(matches!(
        tokens[2].kind,
        TokenKind::CommandSubstitution { .. }
    ));
}
