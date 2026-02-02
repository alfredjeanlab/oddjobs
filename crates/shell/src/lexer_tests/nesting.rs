// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Lexer tests for quote-aware nesting in command substitutions.

use crate::lexer::{Lexer, LexerError};
use crate::token::TokenKind;

#[test]
fn test_double_quote_containing_rparen() {
    // $(echo ")") - the ) inside quotes should not close the substitution
    let tokens = Lexer::tokenize(r#"$(echo ")")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, backtick } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert!(!backtick);
    assert_eq!(content, r#"echo ")""#);
}

#[test]
fn test_single_quote_containing_rparen() {
    // $(echo ')') - the ) inside quotes should not close the substitution
    let tokens = Lexer::tokenize("$(echo ')')").unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, "echo ')'");
}

#[test]
fn test_double_quote_containing_lparen() {
    // $(echo "(") - the ( inside quotes should not increase depth
    let tokens = Lexer::tokenize(r#"$(echo "(")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "(""#);
}

#[test]
fn test_single_quote_containing_lparen() {
    let tokens = Lexer::tokenize("$(echo '(')").unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, "echo '('");
}

#[test]
fn test_nested_substitution_in_quotes() {
    // $(echo "$(inner)") - nested substitution inside quotes
    let tokens = Lexer::tokenize(r#"$(echo "$(inner)")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "$(inner)""#);
}

#[test]
fn test_escaped_double_quote() {
    // $(echo "\"") - escaped double quote
    let tokens = Lexer::tokenize(r#"$(echo "\"")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "\"""#);
}

#[test]
fn test_escaped_backslash() {
    // $(echo \\) - escaped backslash
    let tokens = Lexer::tokenize(r#"$(echo \\)"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo \\"#);
}

#[test]
fn test_single_quote_preserves_backslash() {
    // In single quotes, backslash is literal (not escape)
    let tokens = Lexer::tokenize(r#"$(echo '\\')"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo '\\'"#);
}

#[test]
fn test_mixed_quotes() {
    // $(echo "'" ')' "'")
    let tokens = Lexer::tokenize(r#"$(echo "'" ")" "'")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "'" ")" "'""#);
}

#[test]
fn test_empty_quotes() {
    let tokens = Lexer::tokenize(r#"$(echo "" '')"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "" ''"#);
}

#[test]
fn test_quote_then_paren() {
    // Ensure quotes followed by real parens still work
    let tokens = Lexer::tokenize(r#"$(echo "x" && (subshell))"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "x" && (subshell)"#);
}

#[test]
fn test_complex_nesting() {
    // $(cmd "$(inner ')')") - nested substitution with quotes
    let tokens = Lexer::tokenize(r#"$(cmd "$(inner ')')")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"cmd "$(inner ')')""#);
}

// =============================================================================
// Step 1: Quote Robustness Tests
// =============================================================================

#[test]
fn test_adjacent_quotes_single_double() {
    // 'a'"b" should work - common in shell for mixing quote semantics
    let tokens = Lexer::tokenize(r#"$(echo 'a'"b")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo 'a'"b""#);
}

#[test]
fn test_adjacent_quotes_double_single() {
    let tokens = Lexer::tokenize(r#"$(echo "a"'b')"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "a"'b'"#);
}

#[test]
fn test_empty_quotes_single() {
    let tokens = Lexer::tokenize("$(echo '')").unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, "echo ''");
}

#[test]
fn test_empty_quotes_double() {
    let tokens = Lexer::tokenize(r#"$(echo "")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo """#);
}

#[test]
fn test_quotes_at_substitution_boundary() {
    // Quote starts immediately after $(
    let tokens = Lexer::tokenize(r#"$("arg")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#""arg""#);
}

#[test]
fn test_consecutive_empty_quotes() {
    let tokens = Lexer::tokenize(r#"$(echo ""''"")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo ""''"""#);
}

// =============================================================================
// Step 2: Escape Sequence Coverage Tests
// =============================================================================

#[test]
fn test_escape_backslash_in_double_quotes() {
    // "\\" in double quotes is single backslash
    let tokens = Lexer::tokenize(r#"$(echo "\\")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "\\""#);
}

#[test]
fn test_escape_quote_in_double_quotes() {
    // "\"" is escaped double quote
    let tokens = Lexer::tokenize(r#"$(echo "\"")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "\"""#);
}

#[test]
fn test_escape_dollar_in_double_quotes() {
    // "\$" escapes the dollar sign
    let tokens = Lexer::tokenize(r#"$(echo "\$VAR")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "\$VAR""#);
}

#[test]
fn test_escape_backtick_in_double_quotes() {
    // "\`" escapes backtick
    let tokens = Lexer::tokenize(r#"$(echo "\`cmd\`")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "\`cmd\`""#);
}

#[test]
fn test_literal_backslash_n_in_double_quotes() {
    // "\n" in double quotes is literal backslash-n (NOT newline)
    let tokens = Lexer::tokenize(r#"$(echo "\n")"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo "\n""#);
}

#[test]
fn test_literal_backslash_in_single_quotes() {
    // In single quotes, backslash is always literal
    let tokens = Lexer::tokenize(r#"$(echo '\\')"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo '\\'"#);
}

#[test]
fn test_escaped_paren_outside_quotes() {
    // \) outside quotes should escape the paren
    let tokens = Lexer::tokenize(r#"$(echo \))"#).unwrap();
    assert_eq!(tokens.len(), 1);
    let TokenKind::CommandSubstitution { content, .. } = &tokens[0].kind else {
        panic!("expected command substitution");
    };
    assert_eq!(content, r#"echo \)"#);
}

// =============================================================================
// Step 3: Error Message Quality Tests
// =============================================================================

#[test]
fn test_error_unterminated_single_quote_in_subst() {
    let result = Lexer::tokenize("$(echo 'unterminated)");
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedSubstitution { .. })
    ));
    if let Err(e) = result {
        // Span should point to $( start
        assert_eq!(e.span().start, 0);
    }
}

#[test]
fn test_error_unterminated_double_quote_in_subst() {
    let result = Lexer::tokenize(r#"$(echo "unterminated)"#);
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedSubstitution { .. })
    ));
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
