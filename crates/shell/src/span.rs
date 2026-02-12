// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Source location tracking for shell lexers.

use serde::{Deserialize, Serialize};

/// A byte-offset range in the source text.
///
/// Uses byte offsets for efficient slicing with UTF-8 source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    /// Start byte offset (inclusive)
    pub start: usize,
    /// End byte offset (exclusive)
    pub end: usize,
}

impl Span {
    #[inline]
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "span start must not exceed end");
        Self { start, end }
    }

    #[inline]
    pub fn empty(pos: usize) -> Self {
        Self { start: pos, end: pos }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Returns true if `start <= pos < end`.
    #[inline]
    pub fn contains(&self, pos: usize) -> bool {
        pos >= self.start && pos < self.end
    }

    /// Merge two spans into one that covers both.
    #[inline]
    pub fn merge(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }

    /// Extract the spanned text from source.
    ///
    /// Returns an empty string if the span is out of bounds or not on valid
    /// UTF-8 character boundaries.
    #[inline]
    pub fn slice<'a>(&self, source: &'a str) -> &'a str {
        source.get(self.start..self.end).unwrap_or("")
    }
}

/// Generate a context snippet showing the error location in source text.
///
/// Returns a formatted string with the relevant portion of input and carets
/// pointing to the span location.
///
/// ```text
/// echo | | bad
///        ^^
/// ```
pub fn context_snippet(input: &str, span: Span, context_chars: usize) -> String {
    // Find context boundaries, respecting UTF-8 character boundaries
    let start = input[..span.start]
        .char_indices()
        .rev()
        .take(context_chars)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);

    let end = input[span.start..]
        .char_indices()
        .take(context_chars + 1)
        .last()
        .map(|(i, c)| span.start + i + c.len_utf8())
        .unwrap_or(input.len());

    let snippet = &input[start..end];
    let caret_pos = span.start - start;
    let caret_len = (span.end - span.start).max(1);

    format!("{}\n{}{}", snippet, " ".repeat(caret_pos), "^".repeat(caret_len))
}

/// Locate a span in source, returning (line_number, column, line_content).
///
/// Line numbers are 1-indexed; column is 0-indexed from line start.
pub fn locate_span(source: &str, span: Span) -> (usize, usize, &str) {
    let mut line_num = 1;
    let mut line_start = 0;

    for (i, ch) in source.char_indices() {
        if i >= span.start {
            break;
        }
        if ch == '\n' {
            line_num += 1;
            line_start = i + 1;
        }
    }

    let line_end = source[line_start..].find('\n').map(|i| line_start + i).unwrap_or(source.len());

    // Handle case where span.start might be beyond source length
    let effective_start = span.start.min(source.len());
    let col = if effective_start >= line_start {
        source[line_start..effective_start].chars().count()
    } else {
        0
    };

    let line_content = &source[line_start..line_end];

    (line_num, col, line_content)
}

/// Generate a rich diagnostic message with line/column info.
///
/// Produces output in a format similar to rustc/clippy errors:
///
/// ```text
/// error: unexpected token '|'
///   --> line 3, column 1
///    |
///  3 | | bad
///    | ^
/// ```
pub fn diagnostic_context(source: &str, span: Span, message: &str) -> String {
    let (line_num, col, line_content) = locate_span(source, span);
    let span_len = span.len().max(1);

    format!(
        "error: {}\n  --> line {}, column {}\n   |\n{:>3} | {}\n   | {}{}",
        message,
        line_num,
        col + 1, // 1-indexed for user display
        line_num,
        line_content,
        " ".repeat(col),
        "^".repeat(span_len)
    )
}

#[cfg(test)]
#[path = "span_tests.rs"]
mod tests;
