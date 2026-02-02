// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

// =============================================================================
// locate_span Tests
// =============================================================================

#[test]
fn test_locate_span_single_line() {
    let source = "echo hello world";
    let span = Span::new(5, 10);
    let (line, col, content) = locate_span(source, span);
    assert_eq!(line, 1);
    assert_eq!(col, 5);
    assert_eq!(content, "echo hello world");
}

#[test]
fn test_locate_span_second_line() {
    let source = "echo hello\necho world";
    let span = Span::new(11, 15); // "echo" on line 2
    let (line, col, content) = locate_span(source, span);
    assert_eq!(line, 2);
    assert_eq!(col, 0);
    assert_eq!(content, "echo world");
}

#[test]
fn test_locate_span_third_line() {
    let source = "line one\nline two\nline three";
    let span = Span::new(18, 22); // "line" on line 3
    let (line, col, content) = locate_span(source, span);
    assert_eq!(line, 3);
    assert_eq!(col, 0);
    assert_eq!(content, "line three");
}

#[test]
fn test_locate_span_middle_of_line() {
    let source = "echo hello\nfoo bar baz\nqux";
    let span = Span::new(15, 18); // "bar" on line 2
    let (line, col, content) = locate_span(source, span);
    assert_eq!(line, 2);
    assert_eq!(col, 4);
    assert_eq!(content, "foo bar baz");
}

#[test]
fn test_locate_span_at_newline() {
    let source = "hello\nworld";
    let span = Span::new(5, 6); // The newline character position
    let (line, col, content) = locate_span(source, span);
    // The newline is at the end of line 1
    assert_eq!(line, 1);
    assert_eq!(col, 5);
    assert_eq!(content, "hello");
}

#[test]
fn test_locate_span_empty_line() {
    let source = "hello\n\nworld";
    let span = Span::new(7, 12); // "world" on line 3
    let (line, col, content) = locate_span(source, span);
    assert_eq!(line, 3);
    assert_eq!(col, 0);
    assert_eq!(content, "world");
}

#[test]
fn test_locate_span_unicode() {
    let source = "日本語\nhello";
    let span = Span::new(10, 15); // "hello" after unicode (9 bytes for 3 chars + newline)
    let (line, col, content) = locate_span(source, span);
    assert_eq!(line, 2);
    assert_eq!(col, 0);
    assert_eq!(content, "hello");
}

#[test]
fn test_locate_span_at_start() {
    let source = "hello world";
    let span = Span::new(0, 5);
    let (line, col, content) = locate_span(source, span);
    assert_eq!(line, 1);
    assert_eq!(col, 0);
    assert_eq!(content, "hello world");
}

#[test]
fn test_locate_span_at_end() {
    let source = "hello world";
    let span = Span::new(11, 11); // Empty span at end
    let (line, col, content) = locate_span(source, span);
    assert_eq!(line, 1);
    assert_eq!(col, 11);
    assert_eq!(content, "hello world");
}

#[test]
fn test_locate_span_beyond_source() {
    let source = "hello";
    let span = Span::new(100, 105); // Beyond source length
    let (line, _col, content) = locate_span(source, span);
    // Should return last line info without panicking
    assert_eq!(line, 1);
    assert_eq!(content, "hello");
}

// =============================================================================
// diagnostic_context Tests
// =============================================================================

#[test]
fn test_diagnostic_context_basic() {
    let source = "echo | | bad";
    let span = Span::new(7, 8);
    let diag = diagnostic_context(source, span, "unexpected token '|'");
    assert!(diag.contains("error: unexpected token '|'"));
    assert!(diag.contains("line 1, column 8"));
    assert!(diag.contains("echo | | bad"));
    assert!(diag.contains("^"));
}

#[test]
fn test_diagnostic_context_multiline() {
    let source = "echo hello\necho world\n| bad";
    let span = Span::new(22, 23);
    let diag = diagnostic_context(source, span, "unexpected token '|'");
    assert!(diag.contains("line 3"));
    assert!(diag.contains("| bad"));
}

#[test]
fn test_diagnostic_context_span_length() {
    let source = "echo hello";
    let span = Span::new(5, 10); // "hello" - 5 chars
    let diag = diagnostic_context(source, span, "test");
    assert!(diag.contains("^^^^^")); // 5 carets
}

#[test]
fn test_diagnostic_context_empty_span() {
    let source = "echo hello";
    let span = Span::new(5, 5); // Empty span
    let diag = diagnostic_context(source, span, "test");
    // Should still show at least one caret
    assert!(diag.contains("^"));
}

// =============================================================================
// Property-Based Tests
// =============================================================================

use proptest::prelude::*;

proptest! {
    /// Invariant: locate_span never panics for any valid position.
    #[test]
    fn locate_span_never_panics(
        input in "[a-z\\n ]{1,100}",
        pos in 0usize..50
    ) {
        let pos = pos.min(input.len());
        let span = Span::new(pos, pos.saturating_add(1).min(input.len()));
        let (line, col, content) = locate_span(&input, span);
        // Basic sanity checks
        prop_assert!(line >= 1, "Line number should be >= 1");
        prop_assert!(col <= content.len() + 1, "Column should be within line");
    }

    /// Invariant: diagnostic_context never panics.
    #[test]
    fn diagnostic_context_never_panics(
        input in "[ -~\\n\\t]{0,100}",
        start in 0usize..50,
        len in 0usize..20
    ) {
        let clamped_start = start.min(input.len());
        let clamped_end = (clamped_start + len).min(input.len());
        let span = Span::new(clamped_start, clamped_end);
        let _ = diagnostic_context(&input, span, "test error");
    }

    /// Invariant: context_snippet never panics.
    #[test]
    fn context_snippet_never_panics(
        input in "[ -~\\n\\t]{0,100}",
        start in 0usize..50,
        len in 0usize..20,
        context_chars in 1usize..50
    ) {
        let clamped_start = start.min(input.len());
        let clamped_end = (clamped_start + len).min(input.len());
        let span = Span::new(clamped_start, clamped_end);
        let _ = context_snippet(&input, span, context_chars);
    }

    /// Invariant: locate_span returns consistent line numbers.
    #[test]
    fn locate_span_line_numbers_consistent(input in "[a-z\\n]{5,50}") {
        // Count newlines to get expected line count
        let newline_count = input.matches('\n').count();
        let span = Span::new(input.len().saturating_sub(1), input.len());
        let (line, _, _) = locate_span(&input, span);
        // Line number should be at most newlines + 1
        prop_assert!(line <= newline_count + 1);
    }

    /// Invariant: Span::merge is commutative.
    #[test]
    fn span_merge_commutative(
        s1 in 0usize..100,
        e1 in 0usize..100,
        s2 in 0usize..100,
        e2 in 0usize..100
    ) {
        let span1 = Span::new(s1.min(e1), s1.max(e1));
        let span2 = Span::new(s2.min(e2), s2.max(e2));
        let merged1 = span1.merge(span2);
        let merged2 = span2.merge(span1);
        prop_assert_eq!(merged1, merged2);
    }

    /// Invariant: Span contains checks are consistent.
    #[test]
    fn span_contains_consistent(start in 0usize..50, len in 1usize..50) {
        let span = Span::new(start, start + len);
        // Contains should be true for start <= pos < end
        prop_assert!(span.contains(start));
        prop_assert!(!span.contains(start + len));
        if len > 1 {
            prop_assert!(span.contains(start + 1));
        }
    }
}

// =============================================================================
// Original Span Tests
// =============================================================================

#[test]
fn test_new_span() {
    let span = Span::new(5, 10);
    assert_eq!(span.start, 5);
    assert_eq!(span.end, 10);
}

#[test]
fn test_empty_span() {
    let span = Span::empty(5);
    assert_eq!(span.start, 5);
    assert_eq!(span.end, 5);
    assert!(span.is_empty());
}

#[test]
fn test_len() {
    let span = Span::new(5, 10);
    assert_eq!(span.len(), 5);
}

#[test]
fn test_len_saturates() {
    // This shouldn't happen in practice, but ensure no panic
    let span = Span { start: 10, end: 5 };
    assert_eq!(span.len(), 0);
}

#[test]
fn test_contains() {
    let span = Span::new(5, 10);
    assert!(!span.contains(4));
    assert!(span.contains(5));
    assert!(span.contains(9));
    assert!(!span.contains(10));
}

#[test]
fn test_merge() {
    let span1 = Span::new(5, 10);
    let span2 = Span::new(8, 15);
    let merged = span1.merge(span2);
    assert_eq!(merged.start, 5);
    assert_eq!(merged.end, 15);
}

#[test]
fn test_slice() {
    let source = "echo hello world";
    let span = Span::new(5, 10);
    assert_eq!(span.slice(source), "hello");
}

#[test]
fn test_slice_out_of_bounds() {
    let source = "short";
    let span = Span::new(10, 20);
    assert_eq!(span.slice(source), "");
}
