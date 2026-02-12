// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Parser error types and result structures.

use super::ast::CommandList;
use super::lexer::LexerError;
use super::token::{context_snippet, diagnostic_context, Span, TokenKind};
use thiserror::Error;

/// Parser errors for shell command syntax.
///
/// Use [`ParseError::context`] to generate a human-readable snippet showing
/// where the error occurred.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    #[error("lexer error: {0}")]
    Lexer(#[from] LexerError),

    #[error("unexpected token {found} at position {}, expected {expected}", span.start)]
    UnexpectedToken { found: TokenKind, expected: String, span: Span },

    #[error("unexpected end of input, expected {expected}")]
    UnexpectedEof { expected: String },

    /// Empty command (e.g., just `;`).
    #[error("empty command at position {}", span.start)]
    EmptyCommand { span: Span },

    /// Error inside a `$(...)` or backtick substitution.
    #[error("in command substitution: {inner}")]
    InSubstitution { inner: Box<ParseError>, span: Span },
}

impl ParseError {
    pub fn span(&self) -> Option<Span> {
        match self {
            ParseError::Lexer(e) => Some(e.span()),
            ParseError::UnexpectedToken { span, .. } => Some(*span),
            ParseError::UnexpectedEof { .. } => None,
            ParseError::EmptyCommand { span } => Some(*span),
            ParseError::InSubstitution { span, .. } => Some(*span),
        }
    }

    /// Generate a context snippet showing where the error occurred.
    ///
    /// Returns a string with the relevant portion of input and a caret
    /// pointing to the error location, or `None` if the error has no span.
    pub fn context(&self, input: &str, context_chars: usize) -> Option<String> {
        Some(context_snippet(input, self.span()?, context_chars))
    }

    /// Generate a rich diagnostic with line/column info, or `None` if no span.
    pub fn diagnostic(&self, input: &str) -> Option<String> {
        Some(diagnostic_context(input, self.span()?, &self.to_string()))
    }
}

/// Parse result with potential recovery.
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub commands: CommandList,
    pub errors: Vec<ParseError>,
}
