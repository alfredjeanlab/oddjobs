// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Parser error types and result structures.

use super::ast::CommandList;
use super::lexer::LexerError;
use super::token::{context_snippet, diagnostic_context, Span, TokenKind};
use thiserror::Error;

/// Parser errors.
///
/// These errors indicate problems parsing shell command syntax. Use
/// [`ParseError::context`] to generate a human-readable snippet showing
/// where the error occurred.
///
/// # Examples
///
/// ```
/// use oj_shell::{Parser, ParseError};
///
/// let result = Parser::parse("echo |");
/// assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
///
/// let result = Parser::parse("echo | | bad");
/// assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
/// ```
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Lexer error during tokenization.
    ///
    /// Occurs when the input contains invalid lexical syntax.
    ///
    /// # Example
    /// ```
    /// use oj_shell::{Parser, ParseError, LexerError};
    ///
    /// let result = Parser::parse("echo 'unterminated");
    /// assert!(matches!(result, Err(ParseError::Lexer(LexerError::UnterminatedSingleQuote { .. }))));
    /// ```
    #[error("lexer error: {0}")]
    Lexer(#[from] LexerError),

    /// Unexpected token encountered.
    ///
    /// Occurs when the parser finds a token that doesn't fit the expected grammar.
    ///
    /// # Example
    /// ```
    /// use oj_shell::{Parser, ParseError};
    ///
    /// let result = Parser::parse("echo | | bad");
    /// assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    /// ```
    #[error("unexpected token {found} at position {}, expected {expected}", span.start)]
    UnexpectedToken {
        /// The token that was found.
        found: TokenKind,
        /// Description of what was expected.
        expected: String,
        /// Source location span for the error.
        span: Span,
    },

    /// Unexpected end of input.
    ///
    /// Occurs when more input is needed to complete a command.
    ///
    /// # Example
    /// ```
    /// use oj_shell::{Parser, ParseError};
    ///
    /// let result = Parser::parse("echo &&");
    /// assert!(matches!(result, Err(ParseError::UnexpectedEof { .. })));
    /// ```
    #[error("unexpected end of input, expected {expected}")]
    UnexpectedEof {
        /// Description of what was expected.
        expected: String,
    },

    /// Empty command (e.g., just `;`).
    ///
    /// Occurs when a command is expected but not found.
    ///
    /// # Example
    /// ```
    /// use oj_shell::{Parser, ParseError};
    ///
    /// let result = Parser::parse("| echo");
    /// assert!(matches!(result, Err(ParseError::UnexpectedToken { .. })));
    /// ```
    #[error("empty command at position {}", span.start)]
    EmptyCommand {
        /// Source location span for the error.
        span: Span,
    },

    /// Error inside a command substitution.
    ///
    /// Occurs when the content inside `$(...)` or backticks cannot be parsed.
    ///
    /// # Example
    /// ```
    /// use oj_shell::{Parser, ParseError};
    ///
    /// let result = Parser::parse("echo $(bad |)");
    /// assert!(matches!(result, Err(ParseError::InSubstitution { .. })));
    /// ```
    #[error("in command substitution: {inner}")]
    InSubstitution {
        /// The inner error.
        inner: Box<ParseError>,
        /// Source span of the substitution.
        span: Span,
    },
}

impl ParseError {
    /// Get the span associated with this error, if any.
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
    /// pointing to the error location.
    ///
    /// # Arguments
    ///
    /// * `input` - The original input string that was parsed.
    /// * `context_chars` - Number of characters of context to show around the error.
    ///
    /// # Returns
    ///
    /// `Some(String)` containing the context snippet if the error has a span,
    /// `None` otherwise.
    ///
    /// # Example
    ///
    /// ```text
    /// echo | | bad
    ///        ^
    /// ```
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
    /// Successfully parsed commands.
    pub commands: CommandList,
    /// Errors encountered during parsing.
    pub errors: Vec<ParseError>,
}
