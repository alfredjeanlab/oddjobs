// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Structured execution output and per-command trace records.

use crate::Span;
use std::time::Duration;

/// Outcome of executing a `CommandList`.
#[derive(Debug)]
pub struct ExecOutput {
    /// Exit code of the last command that ran.
    pub exit_code: i32,
    /// Per-command execution traces in order of execution.
    pub traces: Vec<CommandTrace>,
}

/// Record of a single command execution.
#[derive(Debug)]
pub struct CommandTrace {
    /// The command name (argv\[0\]).
    pub command: String,
    /// Full arguments (argv\[1..\]).
    pub args: Vec<String>,
    /// Exit code returned by the process.
    pub exit_code: i32,
    /// Wall-clock duration.
    pub duration: Duration,
    /// First N bytes of captured stdout (if captured).
    pub stdout_snippet: Option<String>,
    /// First N bytes of captured stderr (if captured).
    pub stderr_snippet: Option<String>,
    /// Source span of the AST node that produced this command.
    pub span: Span,
}
