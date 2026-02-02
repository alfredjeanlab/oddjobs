// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Validation error types for shell AST validation.

use crate::{context_snippet, diagnostic_context, Span};
use thiserror::Error;

/// Errors that can occur during AST validation.
///
/// These errors represent semantic problems in successfully parsed ASTs,
/// such as empty structures or excessive nesting.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ValidationError {
    /// Empty command at the given position.
    #[error("empty command at position {}", .span.start)]
    EmptyCommand {
        /// Source location span for the error.
        span: Span,
    },

    /// Missing command after an operator.
    #[error("missing command after `{operator}`")]
    MissingCommandAfter {
        /// The operator that requires a following command.
        operator: String,
        /// Source location span for the error.
        span: Span,
    },

    /// Missing command before an operator.
    #[error("missing command before `{operator}`")]
    MissingCommandBefore {
        /// The operator that requires a preceding command.
        operator: String,
        /// Source location span for the error.
        span: Span,
    },

    /// Empty pipeline segment (no command between pipes).
    #[error("empty pipeline segment")]
    EmptyPipelineSegment {
        /// Source location span for the error.
        span: Span,
    },

    /// Empty subshell `( )`.
    #[error("empty subshell")]
    EmptySubshell {
        /// Source location span for the error.
        span: Span,
    },

    /// Empty brace group `{ }`.
    #[error("empty brace group")]
    EmptyBraceGroup {
        /// Source location span for the error.
        span: Span,
    },

    /// Assignment without a command (when not allowed).
    #[error("assignment without command: `{name}={}`", value.as_deref().unwrap_or(""))]
    StandaloneAssignment {
        /// The variable name.
        name: String,
        /// The assigned value (if any).
        value: Option<String>,
        /// Source location span for the error.
        span: Span,
    },

    /// Redirection without a command.
    #[error("redirection without command")]
    RedirectionWithoutCommand {
        /// Source location span for the error.
        span: Span,
    },

    /// Excessive nesting depth.
    #[error("excessive nesting depth ({depth} levels, max {max})")]
    ExcessiveNesting {
        /// The actual nesting depth.
        depth: usize,
        /// The maximum allowed depth.
        max: usize,
        /// Source location span for the error.
        span: Span,
    },

    /// Attempt to assign to IFS variable.
    #[error("IFS configuration is not supported; word splitting uses default whitespace")]
    IfsAssignment {
        /// Source location span for the error.
        span: Span,
    },
}

impl ValidationError {
    /// Get the span associated with this error.
    pub fn span(&self) -> Span {
        match self {
            Self::EmptyCommand { span } => *span,
            Self::MissingCommandAfter { span, .. } => *span,
            Self::MissingCommandBefore { span, .. } => *span,
            Self::EmptyPipelineSegment { span } => *span,
            Self::EmptySubshell { span } => *span,
            Self::EmptyBraceGroup { span } => *span,
            Self::StandaloneAssignment { span, .. } => *span,
            Self::RedirectionWithoutCommand { span } => *span,
            Self::ExcessiveNesting { span, .. } => *span,
            Self::IfsAssignment { span } => *span,
        }
    }

    /// Get a context snippet around the error position.
    ///
    /// Returns a formatted string showing the error location with a caret.
    ///
    /// # Arguments
    ///
    /// * `input` - The original input string that was parsed.
    /// * `context_chars` - Number of characters of context to show around the error.
    ///
    /// # Example
    ///
    /// ```text
    /// echo hello; ( )
    ///             ^^^
    /// ```
    pub fn context(&self, input: &str, context_chars: usize) -> String {
        context_snippet(input, self.span(), context_chars)
    }

    /// Generate a rich diagnostic with line/column info.
    pub fn diagnostic(&self, input: &str) -> String {
        diagnostic_context(input, self.span(), &self.to_string())
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
