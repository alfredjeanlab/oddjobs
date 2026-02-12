// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command substitution lexer tests: nested $(...), backtick equivalence,
//! depth tracking edge cases, deep nesting stress tests, and AST preservation.
//!
//! ## Design Notes
//!
//! **Depth Tracking:** The lexer tracks ALL parentheses for balance, not just `$()`.
//! For input `$(echo (a))`, depth goes: 1 (for `$(`) → 2 (for `(`) → 1 (for `)`) → 0.
//!
//! **Content Storage:** Command substitution content is stored as raw strings, not
//! recursively parsed to AST. This enables lazy parsing - inner content may never
//! need parsing if the outer command fails.
//!
//! **Depth Limits:** No explicit limit is enforced (uses usize counter). Real shell
//! scripts rarely exceed 3-5 levels, and pathological input is rare in practice.

use crate::lexer::{Lexer, LexerError};
use crate::token::TokenKind;

lex_tests! {
    nested_subst_depth_1: "$(cat $(file))" => [
        TokenKind::CommandSubstitution {
            content: "cat $(file)".into(),
            backtick: false,
        },
    ],
    nested_subst_depth_2: "$(a $(b $(c)))" => [
        TokenKind::CommandSubstitution {
            content: "a $(b $(c))".into(),
            backtick: false,
        },
    ],
    nested_subst_depth_3: "$(a $(b $(c $(d))))" => [
        TokenKind::CommandSubstitution {
            content: "a $(b $(c $(d)))".into(),
            backtick: false,
        },
    ],
    nested_subst_depth_5: "$(a $(b $(c $(d $(e)))))" => [
        TokenKind::CommandSubstitution {
            content: "a $(b $(c $(d $(e))))".into(),
            backtick: false,
        },
    ],
    nested_subst_sibling: "$($(a) $(b))" => [
        TokenKind::CommandSubstitution {
            content: "$(a) $(b)".into(),
            backtick: false,
        },
    ],
    nested_subst_complex_tree: "$(a $(b) $(c $(d)))" => [
        TokenKind::CommandSubstitution {
            content: "a $(b) $(c $(d))".into(),
            backtick: false,
        },
    ],
}

#[yare::parameterized(
    simple    = { "$(date)", "`date`" },
    with_args = { "$(echo hello world)", "`echo hello world`" },
)]
fn backtick_dollar_equivalence(dollar_input: &str, backtick_input: &str) {
    let dollar = Lexer::tokenize(dollar_input).unwrap();
    let backtick = Lexer::tokenize(backtick_input).unwrap();
    assert_eq!(dollar.len(), 1);
    assert_eq!(backtick.len(), 1);
    match (&dollar[0].kind, &backtick[0].kind) {
        (
            TokenKind::CommandSubstitution { content: c1, backtick: false },
            TokenKind::CommandSubstitution { content: c2, backtick: true },
        ) => assert_eq!(c1, c2),
        _ => panic!("Expected CommandSubstitution tokens"),
    }
}

#[test]
fn test_equivalence_adjacent_text() {
    let dollar = Lexer::tokenize("prefix$(cmd)suffix").unwrap();
    let backtick = Lexer::tokenize("prefix`cmd`suffix").unwrap();

    assert_eq!(dollar.len(), 3);
    assert_eq!(backtick.len(), 3);
    assert_eq!(dollar[0].kind, backtick[0].kind);
    assert_eq!(dollar[2].kind, backtick[2].kind);

    match (&dollar[1].kind, &backtick[1].kind) {
        (
            TokenKind::CommandSubstitution { content: c1, .. },
            TokenKind::CommandSubstitution { content: c2, .. },
        ) => assert_eq!(c1, c2),
        _ => panic!("Expected CommandSubstitution tokens"),
    }
}

#[test]
fn test_equivalence_with_operators() {
    let dollar = Lexer::tokenize("$(a) && $(b)").unwrap();
    let backtick = Lexer::tokenize("`a` && `b`").unwrap();

    assert_eq!(dollar.len(), 3);
    assert_eq!(backtick.len(), 3);
    assert_eq!(dollar[1].kind, TokenKind::And);
    assert_eq!(backtick[1].kind, TokenKind::And);
}

lex_tests! {
    backtick_no_nesting: "`a `b` c`" => [
        TokenKind::CommandSubstitution {
            content: "a ".into(),
            backtick: true,
        },
        TokenKind::Word("b".into()),
        TokenKind::CommandSubstitution {
            content: " c".into(),
            backtick: true,
        },
    ],
    mixed_styles_sequence: "$(a) `b` $(c)" => [
        TokenKind::CommandSubstitution {
            content: "a".into(),
            backtick: false,
        },
        TokenKind::CommandSubstitution {
            content: "b".into(),
            backtick: true,
        },
        TokenKind::CommandSubstitution {
            content: "c".into(),
            backtick: false,
        },
    ],
}

lex_tests! {
    depth_parentheses_in_content: "$(echo (a))" => [
        TokenKind::CommandSubstitution {
            content: "echo (a)".into(),
            backtick: false,
        },
    ],
    depth_multiple_parens: "$(test (a) (b) (c))" => [
        TokenKind::CommandSubstitution {
            content: "test (a) (b) (c)".into(),
            backtick: false,
        },
    ],
    depth_nested_parens_no_dollar: "$(echo ((nested)))" => [
        TokenKind::CommandSubstitution {
            content: "echo ((nested))".into(),
            backtick: false,
        },
    ],
    depth_arithmetic_expansion: "$((1+2))" => [
        TokenKind::CommandSubstitution {
            content: "(1+2)".into(),
            backtick: false,
        },
    ],
    empty_substitution: "$()" => [
        TokenKind::CommandSubstitution {
            content: "".into(),
            backtick: false,
        },
    ],
    whitespace_only_substitution: "$(   )" => [
        TokenKind::CommandSubstitution {
            content: "   ".into(),
            backtick: false,
        },
    ],
    unicode_in_substitution: "$(echo 你好)" => [
        TokenKind::CommandSubstitution {
            content: "echo 你好".into(),
            backtick: false,
        },
    ],
    emoji_in_substitution: "$(echo 🦦)" => [
        TokenKind::CommandSubstitution {
            content: "echo 🦦".into(),
            backtick: false,
        },
    ],
    consecutive_substitutions_no_space: "$(a)$(b)$(c)" => [
        TokenKind::CommandSubstitution {
            content: "a".into(),
            backtick: false,
        },
        TokenKind::CommandSubstitution {
            content: "b".into(),
            backtick: false,
        },
        TokenKind::CommandSubstitution {
            content: "c".into(),
            backtick: false,
        },
    ],
}

span_tests! {
    substitution_span_accuracy: "$(cmd)" => [(0, 6)],
    backtick_span_accuracy: "`cmd`" => [(0, 5)],
    nested_span_accuracy: "$(a $(b))" => [(0, 9)],
}

#[test]
fn test_deep_nesting_10_levels() {
    let input = "$(a ".repeat(10) + &")".repeat(10);
    let tokens = Lexer::tokenize(&input).unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(matches!(tokens[0].kind, TokenKind::CommandSubstitution { backtick: false, .. }));
}

lex_tests! {
    deep_nesting_content_preserved: "$(outer $(middle $(inner)))" => [
        TokenKind::CommandSubstitution {
            content: "outer $(middle $(inner))".into(),
            backtick: false,
        },
    ],
    deep_nesting_with_operators: "$(a && $(b || $(c | d)))" => [
        TokenKind::CommandSubstitution {
            content: "a && $(b || $(c | d))".into(),
            backtick: false,
        },
    ],
    deep_nesting_mixed_parens: "$(a (b $(c (d))))" => [
        TokenKind::CommandSubstitution {
            content: "a (b $(c (d)))".into(),
            backtick: false,
        },
    ],
    wide_nesting_many_siblings: "$($(a)$(b)$(c)$(d)$(e))" => [
        TokenKind::CommandSubstitution {
            content: "$(a)$(b)$(c)$(d)$(e)".into(),
            backtick: false,
        },
    ],
}

lex_error_tests! {
    unterminated_at_depth: "$(a $(b $(c)" => LexerError::UnterminatedSubstitution { .. },
    unterminated_inner_only: "$(a $(b)" => LexerError::UnterminatedSubstitution { .. },
    deeply_unterminated: "$(a $(b $(c $(d $(e" => LexerError::UnterminatedSubstitution { .. },
}

#[test]
fn test_nesting_depth_50() {
    let input = "$(a ".repeat(50) + &")".repeat(50);
    let tokens = Lexer::tokenize(&input).unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(matches!(tokens[0].kind, TokenKind::CommandSubstitution { backtick: false, .. }));
}

#[test]
fn test_nesting_depth_100() {
    let input = "$(a ".repeat(100) + &")".repeat(100);
    let tokens = Lexer::tokenize(&input).unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(matches!(tokens[0].kind, TokenKind::CommandSubstitution { backtick: false, .. }));
}

#[test]
fn test_deeply_nested_mixed_constructs() {
    // Alternating $(...) and ${..:-...} nesting
    let mut input = String::new();
    for i in 0..25 {
        if i % 2 == 0 {
            input.push_str("$(a ");
        } else {
            input.push_str("${V:-");
        }
    }
    for i in (0..25).rev() {
        if i % 2 == 0 {
            input.push(')');
        } else {
            input.push('}');
        }
    }
    let result = Lexer::tokenize(&input);
    assert!(result.is_ok(), "Expected success, got: {:?}", result);
}

lex_tests! {
    ast_preserves_nested_content_verbatim: "$(echo $(inner))" => [
        TokenKind::CommandSubstitution {
            content: "echo $(inner)".into(),
            backtick: false,
        },
    ],
    ast_preserves_deeply_nested_content: "$(a $(b $(c)))" => [
        TokenKind::CommandSubstitution {
            content: "a $(b $(c))".into(),
            backtick: false,
        },
    ],
}

lex_error_tests! {
    error_unbalanced_extra_open: "$((a)" => LexerError::UnterminatedSubstitution { .. },
}

#[test]
fn test_error_span_points_to_outermost() {
    let result = Lexer::tokenize("$(a $(b $(c");
    if let Err(LexerError::UnterminatedSubstitution { span }) = result {
        assert_eq!(span.start, 0, "Span should point to outermost $(");
    } else {
        panic!("Expected UnterminatedSubstitution error, got: {:?}", result);
    }
}

lex_error_tests! {
    error_mixed_nesting_unterminated: "$(cmd ${VAR:-" => LexerError::UnterminatedSubstitution { .. },
}

#[test]
fn test_error_context_snippet() {
    let input = "echo $(nested $(unterminated";
    let result = Lexer::tokenize(input);
    if let Err(err) = result {
        let context = err.context(input, 10);
        assert!(context.contains("$("), "Context should include $( marker");
    } else {
        panic!("Expected error");
    }
}
