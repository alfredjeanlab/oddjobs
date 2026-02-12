// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn test_span_extraction() {
    let error = LexerError::EmptyVariable { span: Span::new(5, 6) };
    assert_eq!(error.span(), Span::new(5, 6));
}

#[test]
fn test_context() {
    let input = "echo $ world";
    let error = LexerError::EmptyVariable { span: Span::new(5, 6) };
    let context = error.context(input, 3);
    assert!(context.contains("o $ w"));
    assert!(context.contains("^"));
}

#[test]
fn test_error_display() {
    let error = LexerError::EmptyVariable { span: Span::new(5, 6) };
    let display = format!("{}", error);
    assert!(display.contains("empty variable name"));
    assert!(display.contains("position 5"));
}

#[test]
fn test_diagnostic() {
    let input = "echo $ world";
    let error = LexerError::EmptyVariable { span: Span::new(5, 6) };
    let diag = error.diagnostic(input);
    assert!(diag.contains("error:"));
    assert!(diag.contains("line 1, column 6"));
    assert!(diag.contains("echo $ world"));
    assert!(diag.contains("^"));
}

#[test]
fn test_diagnostic_multiline() {
    let input = "echo hello\necho $ world";
    let error = LexerError::EmptyVariable { span: Span::new(16, 17) };
    let diag = error.diagnostic(input);
    assert!(diag.contains("line 2"));
    assert!(diag.contains("echo $ world"));
}
