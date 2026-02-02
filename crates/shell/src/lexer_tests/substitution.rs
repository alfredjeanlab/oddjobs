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
use crate::token::{Span, TokenKind};

// =============================================================================
// Nested $(...) verification tests
// =============================================================================

#[test]
fn test_nested_subst_depth_1() {
    // Single level nesting
    let tokens = Lexer::tokenize("$(cat $(file))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "cat $(file)".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_nested_subst_depth_2() {
    // Two levels of nesting
    let tokens = Lexer::tokenize("$(a $(b $(c)))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "a $(b $(c))".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_nested_subst_depth_3() {
    // Three levels of nesting
    let tokens = Lexer::tokenize("$(a $(b $(c $(d))))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "a $(b $(c $(d)))".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_nested_subst_depth_5() {
    // Five levels of nesting - stress test
    let tokens = Lexer::tokenize("$(a $(b $(c $(d $(e)))))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "a $(b $(c $(d $(e))))".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_nested_subst_sibling() {
    // Multiple nested substitutions at same level
    let tokens = Lexer::tokenize("$($(a) $(b))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "$(a) $(b)".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_nested_subst_complex_tree() {
    // Complex nesting pattern: $(a $(b) $(c $(d)))
    let tokens = Lexer::tokenize("$(a $(b) $(c $(d)))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "a $(b) $(c $(d))".into(),
            backtick: false,
        }
    );
}

// =============================================================================
// Backtick/dollar-paren equivalence tests
// =============================================================================

#[test]
fn test_equivalence_simple() {
    let dollar = Lexer::tokenize("$(date)").unwrap();
    let backtick = Lexer::tokenize("`date`").unwrap();

    assert_eq!(dollar.len(), 1);
    assert_eq!(backtick.len(), 1);

    // Both have same content
    if let (
        TokenKind::CommandSubstitution {
            content: c1,
            backtick: false,
        },
        TokenKind::CommandSubstitution {
            content: c2,
            backtick: true,
        },
    ) = (&dollar[0].kind, &backtick[0].kind)
    {
        assert_eq!(c1, c2);
        assert_eq!(c1, "date");
    } else {
        panic!("Expected CommandSubstitution tokens");
    }
}

#[test]
fn test_equivalence_with_args() {
    let dollar = Lexer::tokenize("$(echo hello world)").unwrap();
    let backtick = Lexer::tokenize("`echo hello world`").unwrap();

    match (&dollar[0].kind, &backtick[0].kind) {
        (
            TokenKind::CommandSubstitution {
                content: c1,
                backtick: false,
            },
            TokenKind::CommandSubstitution {
                content: c2,
                backtick: true,
            },
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

    // First and last tokens should match
    assert_eq!(dollar[0].kind, backtick[0].kind);
    assert_eq!(dollar[2].kind, backtick[2].kind);

    // Middle tokens should have same content
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

    // Operator should be same
    assert_eq!(dollar[1].kind, TokenKind::And);
    assert_eq!(backtick[1].kind, TokenKind::And);
}

#[test]
fn test_backtick_no_nesting() {
    // Backticks don't support nesting - inner backtick closes outer
    // `a `b` c` should parse as: subst("a ") + Word("b") + subst(" c")
    // This verifies the documented limitation
    let tokens = Lexer::tokenize("`a `b` c`").unwrap();
    // First backtick pair: `a ` -> content "a "
    // Then: b -> Word
    // Then: ` c` -> content " c"
    assert_eq!(tokens.len(), 3);
}

#[test]
fn test_mixed_styles_sequence() {
    let tokens = Lexer::tokenize("$(a) `b` $(c)").unwrap();
    assert_eq!(tokens.len(), 3);

    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "a".into(),
            backtick: false,
        }
    );
    assert_eq!(
        tokens[1].kind,
        TokenKind::CommandSubstitution {
            content: "b".into(),
            backtick: true,
        }
    );
    assert_eq!(
        tokens[2].kind,
        TokenKind::CommandSubstitution {
            content: "c".into(),
            backtick: false,
        }
    );
}

// =============================================================================
// Depth tracking edge case tests
// =============================================================================

#[test]
fn test_depth_parentheses_in_content() {
    // Regular parens (not $(...)) should be counted for balance
    let tokens = Lexer::tokenize("$(echo (a))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "echo (a)".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_depth_multiple_parens() {
    // Multiple paren pairs inside
    let tokens = Lexer::tokenize("$(test (a) (b) (c))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "test (a) (b) (c)".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_depth_nested_parens_no_dollar() {
    // Nested parens without $ prefix
    let tokens = Lexer::tokenize("$(echo ((nested)))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "echo ((nested))".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_depth_arithmetic_expansion() {
    // $((...)) for arithmetic - outer $() contains (...)
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
fn test_empty_substitution() {
    let tokens = Lexer::tokenize("$()").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_whitespace_only_substitution() {
    let tokens = Lexer::tokenize("$(   )").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "   ".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_unicode_in_substitution() {
    let tokens = Lexer::tokenize("$(echo 你好)").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "echo 你好".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_emoji_in_substitution() {
    let tokens = Lexer::tokenize("$(echo 🦦)").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "echo 🦦".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_consecutive_substitutions_no_space() {
    let tokens = Lexer::tokenize("$(a)$(b)$(c)").unwrap();
    assert_eq!(tokens.len(), 3);
    for (i, expected) in ["a", "b", "c"].iter().enumerate() {
        assert_eq!(
            tokens[i].kind,
            TokenKind::CommandSubstitution {
                content: (*expected).into(),
                backtick: false,
            }
        );
    }
}

#[test]
fn test_substitution_span_accuracy() {
    let tokens = Lexer::tokenize("$(cmd)").unwrap();
    assert_eq!(tokens[0].span, Span::new(0, 6)); // $ ( c m d )
}

#[test]
fn test_backtick_span_accuracy() {
    let tokens = Lexer::tokenize("`cmd`").unwrap();
    assert_eq!(tokens[0].span, Span::new(0, 5)); // ` c m d `
}

#[test]
fn test_nested_span_accuracy() {
    let tokens = Lexer::tokenize("$(a $(b))").unwrap();
    assert_eq!(tokens[0].span, Span::new(0, 9)); // Full outer substitution
}

// =============================================================================
// Deep nesting stress tests
// =============================================================================

#[test]
fn test_deep_nesting_10_levels() {
    // Generate: $(a $(a $(a $(a $(a $(a $(a $(a $(a $(a))))))))))
    let input = "$(a ".repeat(10) + &")".repeat(10);
    let tokens = Lexer::tokenize(&input).unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(matches!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            backtick: false,
            ..
        }
    ));
}

#[test]
fn test_deep_nesting_content_preserved() {
    // Verify content at each level is preserved
    let tokens = Lexer::tokenize("$(outer $(middle $(inner)))").unwrap();
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "outer $(middle $(inner))".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_deep_nesting_with_operators() {
    // Deep nesting with shell operators inside
    let tokens = Lexer::tokenize("$(a && $(b || $(c | d)))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "a && $(b || $(c | d))".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_deep_nesting_mixed_parens() {
    // Mix of $() and regular () at depth
    let tokens = Lexer::tokenize("$(a (b $(c (d))))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "a (b $(c (d)))".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_wide_nesting_many_siblings() {
    // Many substitutions at same level
    let tokens = Lexer::tokenize("$($(a)$(b)$(c)$(d)$(e))").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            content: "$(a)$(b)$(c)$(d)$(e)".into(),
            backtick: false,
        }
    );
}

#[test]
fn test_unterminated_at_depth() {
    // Missing closing paren at various depths
    let result = Lexer::tokenize("$(a $(b $(c)");
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedSubstitution { .. })
    ));
}

#[test]
fn test_unterminated_inner_only() {
    // Inner substitution unterminated
    let result = Lexer::tokenize("$(a $(b)");
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedSubstitution { .. })
    ));
}

#[test]
fn test_deeply_unterminated() {
    // Very deep unterminated
    let input = "$(a $(b $(c $(d $(e";
    let result = Lexer::tokenize(input);
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedSubstitution { .. })
    ));
}

// =============================================================================
// Extreme nesting stress tests (Step 2)
// =============================================================================

#[test]
fn test_nesting_depth_50() {
    // 50 levels of command substitution nesting
    let input = "$(a ".repeat(50) + &")".repeat(50);
    let tokens = Lexer::tokenize(&input).unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(matches!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            backtick: false,
            ..
        }
    ));
}

#[test]
fn test_nesting_depth_100() {
    // 100 levels of command substitution nesting
    let input = "$(a ".repeat(100) + &")".repeat(100);
    let tokens = Lexer::tokenize(&input).unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(matches!(
        tokens[0].kind,
        TokenKind::CommandSubstitution {
            backtick: false,
            ..
        }
    ));
}

#[test]
fn test_deeply_nested_mixed_constructs() {
    // Alternating $(...) and ${..:-...} nesting: $( ${ $( ${ ... } ) } )
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

// =============================================================================
// AST content preservation tests (Step 4)
// =============================================================================

#[test]
fn test_ast_preserves_nested_content_verbatim() {
    // Verify nested $() content is stored as raw string, not recursively parsed
    let tokens = Lexer::tokenize("$(echo $(inner))").unwrap();
    assert_eq!(tokens.len(), 1);
    if let TokenKind::CommandSubstitution { content, backtick } = &tokens[0].kind {
        assert_eq!(content, "echo $(inner)");
        assert!(!backtick);
        // The inner $() is NOT parsed - it's just a string
    } else {
        panic!("Expected CommandSubstitution token");
    }
}

#[test]
fn test_ast_preserves_deeply_nested_content() {
    // Three levels of nesting - verify outer content contains inner as raw string
    let tokens = Lexer::tokenize("$(a $(b $(c)))").unwrap();
    assert_eq!(tokens.len(), 1);
    if let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind {
        assert_eq!(content, "a $(b $(c))");
        // Content contains literal "$(b $(c))" - NOT parsed to AST
    } else {
        panic!("Expected CommandSubstitution token");
    }
}

// =============================================================================
// Unbalanced grouping error handling tests (Step 5)
// =============================================================================

#[test]
fn test_error_unbalanced_extra_open() {
    // Double open paren but single close - arithmetic pattern with missing close
    let result = Lexer::tokenize("$((a)");
    assert!(
        matches!(result, Err(LexerError::UnterminatedSubstitution { .. })),
        "Expected UnterminatedSubstitution, got: {:?}",
        result
    );
}

#[test]
fn test_error_span_points_to_outermost() {
    // Verify span starts at the outermost $( that's unterminated
    let result = Lexer::tokenize("$(a $(b $(c");
    if let Err(LexerError::UnterminatedSubstitution { span }) = result {
        assert_eq!(span.start, 0, "Span should point to outermost $(");
    } else {
        panic!("Expected UnterminatedSubstitution error, got: {:?}", result);
    }
}

#[test]
fn test_error_mixed_nesting_unterminated() {
    // $(cmd ${VAR:-   <- missing }} )
    // The lexer processes $( first and treats content as raw string.
    // When EOF is reached, the outer $( is what's unterminated.
    let result = Lexer::tokenize("$(cmd ${VAR:-");
    assert!(
        matches!(result, Err(LexerError::UnterminatedSubstitution { .. })),
        "Expected UnterminatedSubstitution, got: {:?}",
        result
    );
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
