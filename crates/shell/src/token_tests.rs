// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn test_span_merge() {
    let a = Span::new(0, 5);
    let b = Span::new(10, 15);
    let merged = a.merge(b);
    assert_eq!(merged.start, 0);
    assert_eq!(merged.end, 15);
}

#[test]
fn test_span_merge_overlapping() {
    let a = Span::new(0, 10);
    let b = Span::new(5, 15);
    let merged = a.merge(b);
    assert_eq!(merged.start, 0);
    assert_eq!(merged.end, 15);
}

#[test]
fn test_span_contains() {
    let span = Span::new(5, 10);
    assert!(!span.contains(4));
    assert!(span.contains(5));
    assert!(span.contains(7));
    assert!(span.contains(9));
    assert!(!span.contains(10));
}

#[yare::parameterized(
    normal        = { "hello world", 6, 11, "world" },
    out_of_bounds = { "hi",         10, 20, "" },
    unicode       = { "你好世界",     0,  6, "你好" },
)]
fn span_slice(source: &str, start: usize, end: usize, expected: &str) {
    assert_eq!(Span::new(start, end).slice(source), expected);
}
