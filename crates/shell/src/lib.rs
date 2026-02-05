// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! Shell lexer and parser for command-line parsing.
//!
//! This crate provides a complete shell parser that transforms shell command
//! strings into an Abstract Syntax Tree (AST) for analysis and execution.
//!
//! # Quick Start
//!
//! ```ignore
//! use oj_shell::{Parser, CommandList};
//!
//! let ast = Parser::parse("echo hello | grep h")?;
//! println!("Found {} commands", ast.count_simple_commands());
//! # Ok::<(), oj_shell::ParseError>(())
//! ```
//!
//! # Features
//!
//! - **Full shell syntax**: Jobs, AND/OR chains, subshells, brace groups
//! - **Variable expansion**: `$VAR`, `${VAR}`, `${VAR:-default}`
//! - **Command substitution**: `$(cmd)` and `` `cmd` ``
//! - **Quoting**: Single quotes, double quotes, escape sequences
//! - **Redirections**: `>`, `>>`, `<`, `<<`, `<<<`, `&>`, `2>&1`
//!
//! # AST Structure
//!
//! ```text
//! CommandList
//! └── AndOrList[]
//!     └── CommandItem (with background flag)
//!         └── Command (Simple | Job | Subshell | BraceGroup)
//!             └── SimpleCommand
//!                 ├── env: EnvAssignment[]
//!                 ├── name: Word
//!                 └── args: Word[]
//! ```
//!
//! # Parsing
//!
//! Use [`Parser::parse`] for strict parsing that returns an error on invalid input,
//! or [`Parser::parse_with_recovery`] to collect as many valid commands as possible
//! while accumulating errors.
//!
//! # AST Traversal
//!
//! Implement the [`AstVisitor`] trait for custom AST traversal, or use the built-in
//! utility methods on [`CommandList`]:
//!
//! - [`CommandList::count_simple_commands`] - Count all simple commands
//! - [`CommandList::collect_variables`] - Collect all variable references
//! - [`CommandList::has_command_substitutions`] - Check for command substitutions
//! - [`CommandList::max_nesting_depth`] - Get maximum subshell/brace group nesting

// Existing modules from shell-common
mod error;
mod exec;
mod span;
mod validation;

// Shell executor types - re-exported from internal exec module
pub use exec::result::{CommandTrace, ExecOutput};
pub use exec::{ExecError, ShellExecutor};

// Moved modules from runbook::shell
mod ast;
mod lexer;
mod parse_error;
mod parser;
mod token;
mod validator;

// Existing exports from shell-common
pub use error::LexerError;
pub use span::{context_snippet, diagnostic_context, locate_span, Span};
pub use validation::ValidationError;

// AST types
pub use ast::{
    AndOrList, AstVisitor, BraceGroup, CliArg, Command, CommandItem, CommandList, EnvAssignment,
    Job, LogicalOp, QuoteStyle, Redirection, SimpleCommand, Subshell, SubstitutionBody, Word,
    WordPart,
};

// Lexer
pub use lexer::Lexer;

// Parser
pub use parse_error::{ParseError, ParseResult};
pub use parser::Parser;

// Tokens
pub use token::{DupTarget, Token, TokenKind};

// Validator
pub use validator::{validate, validate_with_config, ValidatorConfig};
