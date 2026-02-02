// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Execution error types with span information.

use crate::Span;

/// Errors that can occur during shell command execution.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// A command exited with non-zero status (fail-fast / set -e).
    #[error("command `{command}` failed with exit code {exit_code}")]
    CommandFailed {
        command: String,
        exit_code: i32,
        span: Span,
    },

    /// Command not found or could not be spawned.
    #[error("failed to spawn `{command}`: {source}")]
    SpawnFailed {
        command: String,
        source: std::io::Error,
        span: Span,
    },

    /// Redirection target could not be opened.
    #[error("redirection failed: {message}")]
    RedirectFailed {
        message: String,
        source: std::io::Error,
        span: Span,
    },

    /// Variable expansion failed (required variable missing with no default).
    #[error("undefined variable: ${name}")]
    UndefinedVariable { name: String, span: Span },

    /// A shell feature that is not supported by this executor.
    #[error("unsupported feature: {feature}")]
    Unsupported { feature: String, span: Span },

    /// Glob pattern error (invalid pattern syntax).
    #[error("invalid glob pattern `{pattern}`: {message}")]
    GlobPattern {
        pattern: String,
        message: String,
        span: Span,
    },

    /// Parse error when executor is given a raw string.
    #[error(transparent)]
    Parse(#[from] crate::ParseError),
}

impl ExecError {
    /// Returns the source span associated with this error.
    pub fn span(&self) -> Span {
        match self {
            ExecError::CommandFailed { span, .. }
            | ExecError::SpawnFailed { span, .. }
            | ExecError::RedirectFailed { span, .. }
            | ExecError::UndefinedVariable { span, .. }
            | ExecError::Unsupported { span, .. }
            | ExecError::GlobPattern { span, .. } => *span,
            ExecError::Parse(e) => e.span().unwrap_or_default(),
        }
    }
}
