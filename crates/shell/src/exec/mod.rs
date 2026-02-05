// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Async shell executor that walks a parsed AST and runs commands via
//! [`tokio::process::Command`].
//!
//! The executor provides per-command tracing, structured error reporting with
//! span info, and fine-grained exit code visibility.  Each [`SimpleCommand`]
//! is spawned individually, jobs wire stdout→stdin between processes,
//! and `&&`/`||` chains short-circuit based on exit codes.
//!
//! # Example
//!
//! ```no_run
//! use oj_shell::{Parser, ShellExecutor};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let ast = Parser::parse("echo hello && echo world")?;
//! let result = ShellExecutor::new()
//!     .cwd("/tmp")
//!     .env("PATH", "/usr/bin")
//!     .variable("name", "oddjobs")
//!     .execute(&ast)
//!     .await?;
//!
//! assert_eq!(result.exit_code, 0);
//! assert_eq!(result.traces.len(), 2);
//! # Ok(())
//! # }
//! ```
//!
//! # Unsupported Features
//!
//! The following shell features are **not** supported by this executor:
//!
//! - **Background commands** (`command &`) — returns [`ExecError::Unsupported`]
//! - **Shell builtins** (`cd`, `export`, `source`, `eval`, `trap`, `read`) —
//!   not intercepted; will fail as external commands or succeed if a binary
//!   exists on `PATH`
//! - **Arithmetic expansion** `$((...))` — not in AST, no action needed
//! - **Signal handling / job control** — out of scope
//!
//! # Supported Expansions
//!
//! - **Variable expansion** (`$VAR`, `${VAR:-default}`) — fully supported
//! - **Command substitution** (`$(cmd)`, `` `cmd` ``) — fully supported
//! - **Word splitting** — unquoted variables/substitutions split on IFS
//! - **Glob expansion** (`*`, `?`, `[...]`) — expands against filesystem
//!
//! ## Glob Expansion Notes
//!
//! Glob metacharacters in unquoted literals are expanded against the filesystem.
//! Glob metacharacters from variable expansion or command substitution are NOT
//! expanded (correct POSIX behavior).
//!
//! To use literal metacharacters, use quotes (`'*.txt'` or `"*.txt"`) or
//! backslash escaping (`\*.txt`).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::CommandList;

pub mod error;
mod expand;
mod expand_glob;
mod redirect;
pub mod result;
pub(crate) mod run;

pub use error::ExecError;
use result::ExecOutput;

/// Default snippet capture limit (bytes per stream).
const DEFAULT_SNIPPET_LIMIT: usize = 8192;

/// Executes a parsed shell AST using [`tokio::process::Command`].
///
/// Create an executor with [`ShellExecutor::new`], configure it with builder
/// methods, then call [`execute`](ShellExecutor::execute) or
/// [`execute_str`](ShellExecutor::execute_str).
#[derive(Debug)]
pub struct ShellExecutor {
    cwd: Option<PathBuf>,
    env: HashMap<String, String>,
    variables: HashMap<String, String>,
    /// Max bytes to capture per stream for snippets in [`CommandTrace`].
    snippet_limit: usize,
    /// Enable pipefail semantics for jobs.
    pipefail: bool,
}

impl ShellExecutor {
    /// Create a new executor with default settings.
    pub fn new() -> Self {
        Self {
            cwd: None,
            env: HashMap::new(),
            variables: HashMap::new(),
            snippet_limit: DEFAULT_SNIPPET_LIMIT,
            pipefail: false,
        }
    }

    /// Set the working directory for spawned processes.
    pub fn cwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.cwd = Some(path.into());
        self
    }

    /// Set a single environment variable for spawned processes.
    pub fn env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.insert(key.into(), val.into());
        self
    }

    /// Set multiple environment variables.
    pub fn envs(
        mut self,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (k, v) in vars {
            self.env.insert(k.into(), v.into());
        }
        self
    }

    /// Set a shell variable (for `$VAR` expansion, not passed to processes).
    pub fn variable(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.variables.insert(key.into(), val.into());
        self
    }

    /// Set multiple shell variables.
    pub fn variables(
        mut self,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (k, v) in vars {
            self.variables.insert(k.into(), v.into());
        }
        self
    }

    /// Set the maximum bytes to capture per stream for
    /// [`CommandTrace`] snippets.
    pub fn snippet_limit(mut self, bytes: usize) -> Self {
        self.snippet_limit = bytes;
        self
    }

    /// Enable or disable pipefail mode.
    ///
    /// When enabled, a job returns the exit code of the rightmost
    /// command that failed (non-zero), rather than just the last command.
    /// If all commands succeed, returns 0.
    pub fn pipefail(mut self, enabled: bool) -> Self {
        self.pipefail = enabled;
        self
    }

    /// Execute a parsed AST.
    pub async fn execute(&self, ast: &CommandList) -> Result<ExecOutput, ExecError> {
        let mut ctx = self.build_context()?;
        run::execute_command_list(&mut ctx, ast).await
    }

    /// Parse and execute a shell script string.
    pub async fn execute_str(&self, script: &str) -> Result<ExecOutput, ExecError> {
        let ast = crate::Parser::parse(script)?;
        self.execute(&ast).await
    }

    // Build an `ExecContext` from the current builder state.
    fn build_context(&self) -> Result<run::ExecContext, ExecError> {
        let cwd = match &self.cwd {
            Some(p) => p.clone(),
            None => std::env::current_dir().map_err(|source| ExecError::SpawnFailed {
                command: String::new(),
                source,
                span: crate::Span::default(),
            })?,
        };
        Ok(run::ExecContext {
            cwd,
            env: self.env.clone(),
            variables: self.variables.clone(),
            snippet_limit: self.snippet_limit,
            pipefail: self.pipefail,
            ifs: " \t\n".to_string(),
            last_exit_code: 0, // Initial value before any command runs
        })
    }
}

impl Default for ShellExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../exec_tests/mod.rs"]
mod tests;
