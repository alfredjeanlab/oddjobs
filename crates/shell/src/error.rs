// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Error types for shell lexers.

use crate::{context_snippet, diagnostic_context, Span};
use thiserror::Error;

/// Errors that can occur during shell lexing.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LexerError {
    /// Unexpected character encountered.
    #[error("unexpected character '{ch}' at position {}", span.start)]
    UnexpectedChar {
        /// The unexpected character.
        ch: char,
        /// Source location span for the error.
        span: Span,
    },

    /// Unterminated variable (missing closing brace).
    #[error("unterminated variable at position {}", span.start)]
    UnterminatedVariable {
        /// Source location span for the error.
        span: Span,
    },

    /// Empty variable name (e.g., `$` or `${}`).
    #[error("empty variable name at position {}", span.start)]
    EmptyVariable {
        /// Source location span for the error.
        span: Span,
    },

    /// Invalid variable name (e.g., starts with a digit).
    #[error("invalid variable name '{name}' at position {}", span.start)]
    InvalidVariableName {
        /// The invalid name.
        name: String,
        /// Source location span for the error.
        span: Span,
    },

    /// Unterminated command substitution (missing closing delimiter).
    #[error("unterminated command substitution at position {}", span.start)]
    UnterminatedSubstitution {
        /// Source location span for the error.
        span: Span,
    },

    /// Invalid redirection syntax.
    #[error("invalid redirection: {message} at position {}", span.start)]
    InvalidRedirection {
        /// Description of what's wrong.
        message: String,
        /// Source location span for the error.
        span: Span,
    },

    /// Unterminated single quote.
    #[error("unterminated single quote at position {}", span.start)]
    UnterminatedSingleQuote {
        /// Source location span for the error.
        span: Span,
    },

    /// Unterminated double quote.
    #[error("unterminated double quote at position {}", span.start)]
    UnterminatedDoubleQuote {
        /// Source location span for the error.
        span: Span,
    },

    /// Invalid escape sequence.
    #[error("invalid escape sequence '\\{ch}' at position {}", span.start)]
    InvalidEscape {
        /// The character after the backslash.
        ch: char,
        /// Source location span for the error.
        span: Span,
    },

    /// Trailing backslash at end of input.
    #[error("trailing backslash at position {}", span.start)]
    TrailingBackslash {
        /// Source location span for the error.
        span: Span,
    },

    /// Heredoc was started but never terminated with the delimiter.
    #[error("unterminated here-document at position {}, expected '{delimiter}' delimiter", span.start)]
    UnterminatedHereDoc {
        /// The expected delimiter.
        delimiter: String,
        /// Source location span for the error.
        span: Span,
    },
}

impl LexerError {
    /// Get the span associated with this error.
    pub fn span(&self) -> Span {
        match self {
            Self::UnexpectedChar { span, .. } => *span,
            Self::UnterminatedVariable { span } => *span,
            Self::EmptyVariable { span } => *span,
            Self::InvalidVariableName { span, .. } => *span,
            Self::UnterminatedSubstitution { span } => *span,
            Self::InvalidRedirection { span, .. } => *span,
            Self::UnterminatedSingleQuote { span } => *span,
            Self::UnterminatedDoubleQuote { span } => *span,
            Self::InvalidEscape { span, .. } => *span,
            Self::TrailingBackslash { span } => *span,
            Self::UnterminatedHereDoc { span, .. } => *span,
        }
    }

    /// Get a context snippet around the error position.
    ///
    /// Returns a formatted string showing the error location with a caret.
    pub fn context(&self, input: &str, context_chars: usize) -> String {
        context_snippet(input, self.span(), context_chars)
    }

    /// Generate a rich diagnostic message with line/column info.
    ///
    /// Returns a formatted error message showing the error location with
    /// line numbers, column info, and a caret pointing to the exact position.
    ///
    /// # Example output
    ///
    /// ```text
    /// error: empty variable name at position 5
    ///   --> line 1, column 6
    ///    |
    ///  1 | echo $ world
    ///    |      ^
    /// ```
    pub fn diagnostic(&self, input: &str) -> String {
        diagnostic_context(input, self.span(), &self.to_string())
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
