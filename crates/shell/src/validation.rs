// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Validation error types for shell AST validation.

use crate::{context_snippet, diagnostic_context, Span};
use thiserror::Error;

/// Semantic validation errors for successfully parsed shell ASTs.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("empty command at position {}", .span.start)]
    EmptyCommand { span: Span },

    #[error("missing command after `{operator}`")]
    MissingCommandAfter { operator: String, span: Span },

    #[error("missing command before `{operator}`")]
    MissingCommandBefore { operator: String, span: Span },

    #[error("empty job segment")]
    EmptyJobSegment { span: Span },

    #[error("empty subshell")]
    EmptySubshell { span: Span },

    #[error("empty brace group")]
    EmptyBraceGroup { span: Span },

    #[error("assignment without command: `{name}={}`", value.as_deref().unwrap_or(""))]
    StandaloneAssignment { name: String, value: Option<String>, span: Span },

    #[error("redirection without command")]
    RedirectionWithoutCommand { span: Span },

    #[error("excessive nesting depth ({depth} levels, max {max})")]
    ExcessiveNesting { depth: usize, max: usize, span: Span },

    /// IFS is intentionally unsupported; word splitting always uses default whitespace.
    #[error("IFS configuration is not supported; word splitting uses default whitespace")]
    IfsAssignment { span: Span },
}

impl ValidationError {
    pub fn span(&self) -> Span {
        match self {
            Self::EmptyCommand { span } => *span,
            Self::MissingCommandAfter { span, .. } => *span,
            Self::MissingCommandBefore { span, .. } => *span,
            Self::EmptyJobSegment { span } => *span,
            Self::EmptySubshell { span } => *span,
            Self::EmptyBraceGroup { span } => *span,
            Self::StandaloneAssignment { span, .. } => *span,
            Self::RedirectionWithoutCommand { span } => *span,
            Self::ExcessiveNesting { span, .. } => *span,
            Self::IfsAssignment { span } => *span,
        }
    }

    pub fn context(&self, input: &str, context_chars: usize) -> String {
        context_snippet(input, self.span(), context_chars)
    }

    pub fn diagnostic(&self, input: &str) -> String {
        diagnostic_context(input, self.span(), &self.to_string())
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
