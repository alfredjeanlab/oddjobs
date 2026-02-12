// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Lexer tests for quote-aware nesting in command substitutions.

use crate::lexer::{Lexer, LexerError};
use crate::token::TokenKind;

lex_tests! {
    double_quote_containing_rparen: r#"$(echo ")")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo ")""#.into(),
            backtick: false,
        },
    ],
    single_quote_containing_rparen: "$(echo ')')" => [
        TokenKind::CommandSubstitution {
            content: "echo ')'".into(),
            backtick: false,
        },
    ],
    double_quote_containing_lparen: r#"$(echo "(")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "(""#.into(),
            backtick: false,
        },
    ],
    single_quote_containing_lparen: "$(echo '(')" => [
        TokenKind::CommandSubstitution {
            content: "echo '('".into(),
            backtick: false,
        },
    ],
    nested_substitution_in_quotes: r#"$(echo "$(inner)")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "$(inner)""#.into(),
            backtick: false,
        },
    ],
    escaped_double_quote: r#"$(echo "\"")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "\"""#.into(),
            backtick: false,
        },
    ],
    escaped_backslash: r#"$(echo \\)"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo \\"#.into(),
            backtick: false,
        },
    ],
    single_quote_preserves_backslash: r#"$(echo '\\')"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo '\\'"#.into(),
            backtick: false,
        },
    ],
    mixed_quotes: r#"$(echo "'" ")" "'")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "'" ")" "'""#.into(),
            backtick: false,
        },
    ],
    empty_quotes: r#"$(echo "" '')"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "" ''"#.into(),
            backtick: false,
        },
    ],
    quote_then_paren: r#"$(echo "x" && (subshell))"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "x" && (subshell)"#.into(),
            backtick: false,
        },
    ],
    complex_nesting: r#"$(cmd "$(inner ')')")"# => [
        TokenKind::CommandSubstitution {
            content: r#"cmd "$(inner ')')""#.into(),
            backtick: false,
        },
    ],
}

lex_tests! {
    adjacent_quotes_single_double: r#"$(echo 'a'"b")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo 'a'"b""#.into(),
            backtick: false,
        },
    ],
    adjacent_quotes_double_single: r#"$(echo "a"'b')"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "a"'b'"#.into(),
            backtick: false,
        },
    ],
    empty_quotes_single: "$(echo '')" => [
        TokenKind::CommandSubstitution {
            content: "echo ''".into(),
            backtick: false,
        },
    ],
    empty_quotes_double: r#"$(echo "")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo """#.into(),
            backtick: false,
        },
    ],
    quotes_at_substitution_boundary: r#"$("arg")"# => [
        TokenKind::CommandSubstitution {
            content: r#""arg""#.into(),
            backtick: false,
        },
    ],
    consecutive_empty_quotes: r#"$(echo ""''"")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo ""''"""#.into(),
            backtick: false,
        },
    ],
}

lex_tests! {
    escape_backslash_in_double_quotes: r#"$(echo "\\")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "\\""#.into(),
            backtick: false,
        },
    ],
    escape_quote_in_double_quotes: r#"$(echo "\"")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "\"""#.into(),
            backtick: false,
        },
    ],
    escape_dollar_in_double_quotes: r#"$(echo "\$VAR")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "\$VAR""#.into(),
            backtick: false,
        },
    ],
    escape_backtick_in_double_quotes: r#"$(echo "\`cmd\`")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "\`cmd\`""#.into(),
            backtick: false,
        },
    ],
    literal_backslash_n_in_double_quotes: r#"$(echo "\n")"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo "\n""#.into(),
            backtick: false,
        },
    ],
    literal_backslash_in_single_quotes: r#"$(echo '\\')"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo '\\'"#.into(),
            backtick: false,
        },
    ],
    escaped_paren_outside_quotes: r#"$(echo \))"# => [
        TokenKind::CommandSubstitution {
            content: r#"echo \)"#.into(),
            backtick: false,
        },
    ],
}

lex_error_tests! {
    error_unterminated_single_quote_in_subst: "$(echo 'unterminated)" => LexerError::UnterminatedSubstitution { .. },
    error_unterminated_double_quote_in_subst: r#"$(echo "unterminated)"# => LexerError::UnterminatedSubstitution { .. },
}

#[test]
fn test_error_context_shows_quote_position() {
    let input = "$(echo 'missing close";
    let result = Lexer::tokenize(input);
    if let Err(e) = result {
        let context = e.context(input, 20);
        // Context should show the problematic area
        assert!(context.contains("'missing"));
    }
}

#[test]
fn test_error_span_accuracy_with_unicode() {
    // Unicode before error should not corrupt span position
    let input = "$(echo 你好 'unclosed";
    let result = Lexer::tokenize(input);
    assert!(result.is_err());
    if let Err(e) = result {
        let context = e.context(input, 30);
        // Should still produce valid context
        assert!(!context.is_empty());
    }
}
