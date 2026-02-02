// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Word and text expansion: variables, literals, command substitution, glob, and tilde.
//!
//! This module provides two main expansion modes:
//! - **Word expansion** (`expand_word`, `expand_word_split_glob`): Operates on parsed `Word` AST nodes
//! - **Text expansion** (`expand_heredoc_body`): Operates on raw strings (heredocs, modifier defaults)

mod modifier;
mod text;
mod word;

use crate::{Parser, SubstitutionBody};

use super::error::ExecError;
use super::run::ExecContext;

// Re-export public API
pub(crate) use text::expand_heredoc_body;
pub(crate) use word::{expand_word, expand_word_split_glob};

/// Execute a command substitution and capture its output.
pub(super) async fn execute_substitution(
    ctx: &mut ExecContext,
    body: &SubstitutionBody,
) -> Result<String, ExecError> {
    match body {
        SubstitutionBody::Parsed(ast) => {
            let mut sub_ctx = ctx.clone();
            super::run::execute_command_list_capture(&mut sub_ctx, ast).await
        }
        SubstitutionBody::Unparsed(text) => {
            let ast = Parser::parse(text)?;
            let mut sub_ctx = ctx.clone();
            super::run::execute_command_list_capture(&mut sub_ctx, &ast).await
        }
    }
}
