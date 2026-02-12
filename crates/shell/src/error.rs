// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Error types for shell lexers.

use crate::{context_snippet, diagnostic_context, Span};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LexerError {
    #[error("unexpected character '{ch}' at position {}", span.start)]
    UnexpectedChar { ch: char, span: Span },

    #[error("unterminated variable at position {}", span.start)]
    UnterminatedVariable { span: Span },

    #[error("empty variable name at position {}", span.start)]
    EmptyVariable { span: Span },

    #[error("invalid variable name '{name}' at position {}", span.start)]
    InvalidVariableName { name: String, span: Span },

    #[error("unterminated command substitution at position {}", span.start)]
    UnterminatedSubstitution { span: Span },

    #[error("invalid redirection: {message} at position {}", span.start)]
    InvalidRedirection { message: String, span: Span },

    #[error("unterminated single quote at position {}", span.start)]
    UnterminatedSingleQuote { span: Span },

    #[error("unterminated double quote at position {}", span.start)]
    UnterminatedDoubleQuote { span: Span },

    #[error("invalid escape sequence '\\{ch}' at position {}", span.start)]
    InvalidEscape { ch: char, span: Span },

    #[error("trailing backslash at position {}", span.start)]
    TrailingBackslash { span: Span },

    #[error("unterminated here-document at position {}, expected '{delimiter}' delimiter", span.start)]
    UnterminatedHereDoc { delimiter: String, span: Span },
}

impl LexerError {
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

    pub fn context(&self, input: &str, context_chars: usize) -> String {
        context_snippet(input, self.span(), context_chars)
    }

    /// Generate a rich diagnostic message with line/column info and caret.
    pub fn diagnostic(&self, input: &str) -> String {
        diagnostic_context(input, self.span(), &self.to_string())
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
