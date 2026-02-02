// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Text expansion: expands raw strings (heredoc bodies, modifier defaults) with
//! variable substitution and command substitution.

use crate::{Span, SubstitutionBody};

use super::super::error::ExecError;
use super::super::run::ExecContext;
use super::modifier::apply_modifier;

/// Expand variables and command substitutions in a heredoc body.
///
/// Handles `$VAR`, `${VAR}`, `${VAR:-default}`, `$(cmd)`, and `` `cmd` ``.
pub(crate) async fn expand_heredoc_body(
    ctx: &mut ExecContext,
    body: &str,
) -> Result<String, ExecError> {
    let mut result = String::with_capacity(body.len());
    let mut chars = body.char_indices().peekable();

    while let Some((pos, ch)) = chars.next() {
        if ch == '$' {
            match chars.peek().map(|(_, c)| *c) {
                Some('{') => {
                    chars.next();
                    let expanded = expand_braced_var(&mut chars, ctx, pos).await?;
                    result.push_str(&expanded);
                }
                Some('(') => {
                    chars.next();
                    let expanded = expand_command_subst(&mut chars, ctx).await?;
                    result.push_str(&expanded);
                }
                Some(c) if is_valid_variable_start(c) => {
                    let expanded = expand_simple_var(&mut chars, ctx, pos)?;
                    result.push_str(&expanded);
                }
                _ => result.push('$'),
            }
        } else if ch == '`' {
            let expanded = expand_backtick_subst(&mut chars, ctx).await?;
            result.push_str(&expanded);
        } else if ch == '\\' {
            match chars.peek().map(|(_, c)| *c) {
                Some('$') | Some('`') | Some('\\') => {
                    if let Some((_, escaped)) = chars.next() {
                        result.push(escaped);
                    }
                }
                Some('\n') => {
                    chars.next();
                }
                _ => result.push('\\'),
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

fn is_valid_variable_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_valid_variable_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Expand a simple variable reference ($VAR).
fn expand_simple_var(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    ctx: &ExecContext,
    start_pos: usize,
) -> Result<String, ExecError> {
    let mut name = String::new();
    while let Some(&(_, ch)) = chars.peek() {
        if is_valid_variable_char(ch) {
            name.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    ctx.variables
        .get(&name)
        .cloned()
        .ok_or_else(|| ExecError::UndefinedVariable {
            name,
            span: Span::new(start_pos, start_pos),
        })
}

/// Expand a braced variable reference (${VAR} or ${VAR:-default}).
async fn expand_braced_var(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    ctx: &mut ExecContext,
    start_pos: usize,
) -> Result<String, ExecError> {
    let mut name = String::new();
    let mut modifier = String::new();
    let mut in_modifier = false;
    let mut depth = 1;

    for (_, ch) in chars.by_ref() {
        match ch {
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let value = ctx.variables.get(&name).cloned();
                    return if in_modifier {
                        apply_modifier(
                            ctx,
                            &name,
                            &modifier,
                            value.as_deref(),
                            Span::new(start_pos, start_pos),
                        )
                        .await
                    } else {
                        value.ok_or_else(|| ExecError::UndefinedVariable {
                            name: name.clone(),
                            span: Span::new(start_pos, start_pos),
                        })
                    };
                }
                if in_modifier {
                    modifier.push(ch);
                }
            }
            '{' => {
                depth += 1;
                if in_modifier {
                    modifier.push(ch);
                }
            }
            ':' | '-' | '+' | '=' | '?' if !in_modifier && depth == 1 => {
                in_modifier = true;
                modifier.push(ch);
            }
            _ => {
                if in_modifier {
                    modifier.push(ch);
                } else if is_valid_variable_char(ch) {
                    name.push(ch);
                } else {
                    in_modifier = true;
                    modifier.push(ch);
                }
            }
        }
    }

    let mut literal = String::from("${");
    literal.push_str(&name);
    if in_modifier {
        literal.push_str(&modifier);
    }
    Ok(literal)
}

/// Expand a command substitution ($(cmd)).
async fn expand_command_subst(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    ctx: &mut ExecContext,
) -> Result<String, ExecError> {
    let mut content = String::new();
    let mut depth = 1;

    for (_, ch) in chars.by_ref() {
        match ch {
            '(' => {
                depth += 1;
                content.push(ch);
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let output =
                        super::execute_substitution(ctx, &SubstitutionBody::Unparsed(content))
                            .await?;
                    return Ok(output.trim_end_matches('\n').to_string());
                }
                content.push(ch);
            }
            _ => content.push(ch),
        }
    }

    Ok(format!("$({content}"))
}

/// Expand a backtick command substitution (`cmd`).
async fn expand_backtick_subst(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    ctx: &mut ExecContext,
) -> Result<String, ExecError> {
    let mut content = String::new();

    while let Some((_, ch)) = chars.next() {
        if ch == '`' {
            let output =
                super::execute_substitution(ctx, &SubstitutionBody::Unparsed(content)).await?;
            return Ok(output.trim_end_matches('\n').to_string());
        } else if ch == '\\' {
            if let Some(&(_, next)) = chars.peek() {
                if next == '`' || next == '\\' || next == '$' {
                    chars.next();
                    content.push(next);
                    continue;
                }
            }
            content.push(ch);
        } else {
            content.push(ch);
        }
    }

    Ok(format!("`{content}"))
}
