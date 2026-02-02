// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Lexer tests split into logical modules to stay under line limits.

#[macro_use]
mod macros;

mod assignment;
mod basic;
mod envprefix;
mod errors;
mod expansions;
mod heredoc;
mod nesting;
mod quoting;
mod redirection;
mod substitution;
mod variables;
