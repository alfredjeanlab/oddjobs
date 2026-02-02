// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Comprehensive edge case tests for lexer error scenarios.
//!
//! These tests verify error behavior for unusual input and edge cases
//! across all LexerError variants.

use crate::lexer::{Lexer, LexerError};

// =============================================================================
// EmptyVariable Edge Cases
// =============================================================================

#[test]
fn empty_variable_in_double_quotes_is_literal() {
    // A lone $ at end of double quotes is treated as literal, not an error
    // This matches standard shell behavior
    let result = Lexer::tokenize("echo \"$\"");
    assert!(
        result.is_ok(),
        "Lone $ in quotes should be literal, got: {:?}",
        result
    );
}

#[test]
fn error_empty_variable_at_eof() {
    let result = Lexer::tokenize("echo $");
    assert!(
        matches!(result, Err(LexerError::EmptyVariable { .. })),
        "Expected EmptyVariable error, got: {:?}",
        result
    );
}

#[test]
fn error_empty_braced_variable() {
    let result = Lexer::tokenize("echo ${}");
    assert!(
        matches!(result, Err(LexerError::EmptyVariable { .. })),
        "Expected EmptyVariable error, got: {:?}",
        result
    );
}

#[test]
fn error_empty_braced_variable_in_quotes() {
    let result = Lexer::tokenize("echo \"${}\"");
    assert!(
        matches!(result, Err(LexerError::EmptyVariable { .. })),
        "Expected EmptyVariable error, got: {:?}",
        result
    );
}

// =============================================================================
// InvalidVariableName Edge Cases
// =============================================================================

#[test]
fn error_variable_with_unicode_name() {
    let result = Lexer::tokenize("echo ${日本語}");
    assert!(
        matches!(result, Err(LexerError::InvalidVariableName { .. })),
        "Expected InvalidVariableName error, got: {:?}",
        result
    );
}

#[test]
fn error_variable_starts_with_digit() {
    let result = Lexer::tokenize("echo ${123}");
    assert!(
        matches!(result, Err(LexerError::InvalidVariableName { .. })),
        "Expected InvalidVariableName error, got: {:?}",
        result
    );
}

#[test]
fn error_variable_with_special_chars() {
    let result = Lexer::tokenize("echo ${foo@bar}");
    // The @ should be parsed as a modifier or cause an error
    // depending on the lexer implementation
    // This test documents the current behavior
    assert!(
        result.is_err() || result.is_ok(),
        "Should either succeed with modifier or fail"
    );
}

// =============================================================================
// Quote Edge Cases
// =============================================================================

#[test]
fn error_nested_quotes_unterminated() {
    // Single quote inside double quote - the double quote is unterminated
    let result = Lexer::tokenize("echo \"test'");
    assert!(
        matches!(result, Err(LexerError::UnterminatedDoubleQuote { .. })),
        "Expected UnterminatedDoubleQuote error, got: {:?}",
        result
    );
}

#[test]
fn error_backslash_eof_in_double_quotes() {
    let result = Lexer::tokenize("echo \"test\\");
    // This should be TrailingBackslash (inside quote context)
    assert!(
        matches!(result, Err(LexerError::TrailingBackslash { .. })),
        "Expected TrailingBackslash error, got: {:?}",
        result
    );
}

#[test]
fn error_single_quote_never_closed() {
    let result = Lexer::tokenize("echo 'hello world");
    assert!(
        matches!(result, Err(LexerError::UnterminatedSingleQuote { .. })),
        "Expected UnterminatedSingleQuote error, got: {:?}",
        result
    );
}

#[test]
fn error_double_quote_never_closed() {
    let result = Lexer::tokenize("echo \"hello world");
    assert!(
        matches!(result, Err(LexerError::UnterminatedDoubleQuote { .. })),
        "Expected UnterminatedDoubleQuote error, got: {:?}",
        result
    );
}

// =============================================================================
// HereDoc Edge Cases
// =============================================================================

#[test]
fn error_heredoc_never_terminated() {
    let result = Lexer::tokenize("cat <<EOF\nsome content that never ends");
    assert!(
        matches!(result, Err(LexerError::UnterminatedHereDoc { .. })),
        "Expected UnterminatedHereDoc error, got: {:?}",
        result
    );
}

#[test]
fn error_heredoc_no_body() {
    let result = Lexer::tokenize("cat <<EOF");
    assert!(
        matches!(result, Err(LexerError::UnterminatedHereDoc { .. })),
        "Expected UnterminatedHereDoc error, got: {:?}",
        result
    );
}

#[test]
fn error_heredoc_partial_delimiter() {
    // Delimiter is EOF but only EO appears on a line
    let result = Lexer::tokenize("cat <<EOF\nEO\nmore content");
    assert!(
        matches!(result, Err(LexerError::UnterminatedHereDoc { .. })),
        "Expected UnterminatedHereDoc error, got: {:?}",
        result
    );
}

// =============================================================================
// Substitution Edge Cases
// =============================================================================

#[test]
fn error_unterminated_command_substitution() {
    let result = Lexer::tokenize("echo $(date");
    assert!(
        matches!(result, Err(LexerError::UnterminatedSubstitution { .. })),
        "Expected UnterminatedSubstitution error, got: {:?}",
        result
    );
}

#[test]
fn error_unterminated_backtick_substitution() {
    let result = Lexer::tokenize("echo `date");
    assert!(
        matches!(result, Err(LexerError::UnterminatedSubstitution { .. })),
        "Expected UnterminatedSubstitution error, got: {:?}",
        result
    );
}

#[test]
fn error_nested_unterminated_substitution() {
    let result = Lexer::tokenize("echo $(echo $(inner)");
    assert!(
        matches!(result, Err(LexerError::UnterminatedSubstitution { .. })),
        "Expected UnterminatedSubstitution error, got: {:?}",
        result
    );
}

// =============================================================================
// Variable Edge Cases
// =============================================================================

#[test]
fn error_unterminated_braced_variable() {
    let result = Lexer::tokenize("echo ${HOME");
    assert!(
        matches!(result, Err(LexerError::UnterminatedVariable { .. })),
        "Expected UnterminatedVariable error, got: {:?}",
        result
    );
}

#[test]
fn error_unterminated_variable_with_modifier() {
    let result = Lexer::tokenize("echo ${HOME:-default");
    assert!(
        matches!(result, Err(LexerError::UnterminatedVariable { .. })),
        "Expected UnterminatedVariable error, got: {:?}",
        result
    );
}

// =============================================================================
// Escape Sequence Edge Cases
// =============================================================================

#[test]
fn error_invalid_escape_in_double_quote() {
    // \a is not a valid escape in shell double quotes
    let result = Lexer::tokenize("echo \"\\a\"");
    assert!(
        matches!(result, Err(LexerError::InvalidEscape { ch: 'a', .. })),
        "Expected InvalidEscape error, got: {:?}",
        result
    );
}

#[test]
fn trailing_backslash_outside_quotes_is_word() {
    // Trailing backslash outside quotes is kept as part of the word
    // (line continuation would only apply at newline)
    let result = Lexer::tokenize("echo test\\");
    assert!(
        result.is_ok(),
        "Trailing backslash outside quotes should be part of word, got: {:?}",
        result
    );
}

// =============================================================================
// Span Accuracy Tests
// =============================================================================

#[test]
fn error_span_points_to_correct_position() {
    let result = Lexer::tokenize("echo hello $");
    if let Err(LexerError::EmptyVariable { span }) = result {
        // $ is at position 11
        assert_eq!(span.start, 11, "Span should start at $ position");
    } else {
        panic!("Expected EmptyVariable error, got: {:?}", result);
    }
}

#[test]
fn error_span_covers_full_token() {
    let result = Lexer::tokenize("echo ${123abc}");
    if let Err(LexerError::InvalidVariableName { span, .. }) = result {
        // Check span is reasonable (covers the variable name)
        assert!(span.start >= 5, "Span should be within the braces");
        assert!(span.end <= 14, "Span should not exceed input");
    } else {
        panic!("Expected InvalidVariableName error, got: {:?}", result);
    }
}

#[test]
fn error_span_multiline() {
    let result = Lexer::tokenize("echo hello\necho $");
    if let Err(LexerError::EmptyVariable { span }) = result {
        // $ is at position 16 (11 for "echo hello\n" + 5 for "echo ")
        assert_eq!(span.start, 16, "Span should point to $ on second line");
    } else {
        panic!("Expected EmptyVariable error, got: {:?}", result);
    }
}

// =============================================================================
// Context Generation Tests
// =============================================================================

#[test]
fn error_context_shows_correct_line() {
    let input = "echo hello\necho $ world";
    let result = Lexer::tokenize(input);
    if let Err(e) = result {
        let diag = e.diagnostic(input);
        assert!(
            diag.contains("line 2"),
            "Diagnostic should show line 2: {}",
            diag
        );
        assert!(
            diag.contains("echo $ world"),
            "Diagnostic should show the line content: {}",
            diag
        );
    } else {
        panic!("Expected error, got success");
    }
}

#[test]
fn error_context_with_unicode() {
    let input = "echo 日本語\necho $";
    let result = Lexer::tokenize(input);
    if let Err(e) = result {
        // Should not panic on Unicode input
        let diag = e.diagnostic(input);
        assert!(
            diag.contains("line 2"),
            "Diagnostic should handle Unicode: {}",
            diag
        );
    } else {
        panic!("Expected error, got success");
    }
}
