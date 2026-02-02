// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Variable modifier parsing and application (${VAR:-default}, ${VAR:=value}, etc.).

use std::future::Future;
use std::pin::Pin;

use crate::Span;

use super::super::error::ExecError;
use super::super::run::ExecContext;
use super::text::expand_heredoc_body;

/// Parse a modifier string into (check_empty, operator, default_value).
///
/// Modifiers with a leading colon (`:- := :+ :?`) trigger on both unset and
/// empty values. Without the colon (`- = + ?`), they trigger only on truly
/// unset variables.
fn parse_modifier(modifier: &str) -> (bool, char, &str) {
    let bytes = modifier.as_bytes();
    if bytes.first() == Some(&b':') {
        let op = bytes.get(1).copied().unwrap_or(b'-') as char;
        let val = if modifier.len() > 2 {
            &modifier[2..]
        } else {
            ""
        };
        (true, op, val)
    } else {
        let op = bytes.first().copied().unwrap_or(b'-') as char;
        let val = if modifier.len() > 1 {
            &modifier[1..]
        } else {
            ""
        };
        (false, op, val)
    }
}

/// Apply a variable modifier, expanding any variables or command substitutions in the default.
pub(super) fn apply_modifier<'a>(
    ctx: &'a mut ExecContext,
    name: &'a str,
    modifier: &'a str,
    value: Option<&'a str>,
    span: Span,
) -> Pin<Box<dyn Future<Output = Result<String, ExecError>> + 'a>> {
    Box::pin(async move {
        let (check_empty, op, default_val) = parse_modifier(modifier);
        let is_unset = value.is_none();
        let should_apply = if check_empty {
            is_unset || value.is_some_and(|v| v.is_empty())
        } else {
            is_unset
        };

        let is_special = matches!(name, "?" | "$" | "#" | "0");

        match op {
            '-' => {
                if should_apply {
                    expand_heredoc_body(ctx, default_val).await
                } else {
                    Ok(value.unwrap_or_default().to_string())
                }
            }
            '=' => {
                if should_apply {
                    let expanded = expand_heredoc_body(ctx, default_val).await?;
                    if !is_special {
                        ctx.variables.insert(name.to_string(), expanded.clone());
                    }
                    Ok(expanded)
                } else {
                    Ok(value.unwrap_or_default().to_string())
                }
            }
            '+' => {
                if should_apply {
                    Ok(String::new())
                } else {
                    expand_heredoc_body(ctx, default_val).await
                }
            }
            '?' => {
                if should_apply {
                    Err(ExecError::UndefinedVariable {
                        name: name.to_string(),
                        span,
                    })
                } else {
                    Ok(value.unwrap_or_default().to_string())
                }
            }
            _ => Ok(value.unwrap_or_default().to_string()),
        }
    })
}
