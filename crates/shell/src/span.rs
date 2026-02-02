// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Source location tracking for shell lexers.

use serde::{Deserialize, Serialize};

/// A span representing a range in the source text.
///
/// Spans use byte offsets for efficient slicing and work with UTF-8 source.
///
/// # Examples
///
/// ```
/// use oj_shell::Span;
///
/// let source = "echo hello";
/// let span = Span::new(5, 10);
/// assert_eq!(span.slice(source), "hello");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Span {
    /// Start byte offset (inclusive)
    pub start: usize,
    /// End byte offset (exclusive)
    pub end: usize,
}

impl Span {
    /// Create a new span from start to end byte positions.
    #[inline]
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "span start must not exceed end");
        Self { start, end }
    }

    /// Create an empty span at a position.
    #[inline]
    pub fn empty(pos: usize) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }

    /// Returns the length of the span in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns true if the span is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Check if this span contains a byte position.
    ///
    /// Returns true if `start <= pos < end`.
    #[inline]
    pub fn contains(&self, pos: usize) -> bool {
        pos >= self.start && pos < self.end
    }

    /// Merge two spans into one that covers both.
    #[inline]
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
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
/// # Arguments
///
/// * `input` - The original input string.
/// * `span` - The span to highlight.
/// * `context_chars` - Number of characters of context to show around the span.
///
/// # Example
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

    format!(
        "{}\n{}{}",
        snippet,
        " ".repeat(caret_pos),
        "^".repeat(caret_len)
    )
}

/// Locate a span in source, returning (line_number, column, line_content).
///
/// Line numbers are 1-indexed (first line is line 1).
/// Column is 0-indexed from line start (first char is column 0).
///
/// # Arguments
///
/// * `source` - The original source text.
/// * `span` - The span to locate.
///
/// # Returns
///
/// A tuple of (line_number, column, line_content) where:
/// - `line_number` is the 1-indexed line number
/// - `column` is the 0-indexed column within the line
/// - `line_content` is the content of the line (without trailing newline)
///
/// # Example
///
/// ```
/// use oj_shell::{Span, locate_span};
///
/// let source = "echo hello\necho world";
/// let span = Span::new(11, 15); // "echo" on line 2
/// let (line, col, content) = locate_span(source, span);
/// assert_eq!(line, 2);
/// assert_eq!(col, 0);
/// assert_eq!(content, "echo world");
/// ```
pub fn locate_span(source: &str, span: Span) -> (usize, usize, &str) {
    let mut line_num = 1;
    let mut line_start = 0;

    // Find which line contains span.start
    for (i, ch) in source.char_indices() {
        if i >= span.start {
            break;
        }
        if ch == '\n' {
            line_num += 1;
            line_start = i + 1;
        }
    }

    // Find the end of the current line
    let line_end = source[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(source.len());

    // Calculate column as character count from line start to span start
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
///
/// # Arguments
///
/// * `source` - The original source text.
/// * `span` - The span to highlight.
/// * `message` - The error message to display.
///
/// # Example
///
/// ```
/// use oj_shell::{Span, diagnostic_context};
///
/// let source = "echo | | bad";
/// let span = Span::new(7, 8);
/// let diag = diagnostic_context(source, span, "unexpected token '|'");
/// assert!(diag.contains("line 1, column 8"));
/// assert!(diag.contains("echo | | bad"));
/// ```
pub fn diagnostic_context(source: &str, span: Span, message: &str) -> String {
    let (line_num, col, line_content) = locate_span(source, span);
    let span_len = span.len().max(1);

    // Format with line number gutter
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
