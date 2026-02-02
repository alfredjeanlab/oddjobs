// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Word expansion: expands parsed `Word` AST nodes with variable substitution,
//! tilde expansion, word splitting, and glob expansion.

use crate::{QuoteStyle, Word, WordPart};

use super::super::error::ExecError;
use super::super::expand_glob::{
    expand_glob_pattern, process_glob_escapes, GlobConfig, GlobEligibility,
};
use super::super::run::ExecContext;
use super::modifier::apply_modifier;

// ---------------------------------------------------------------------------
// Tilde expansion
// ---------------------------------------------------------------------------

/// Expand tilde prefix in a word.
fn expand_tilde(text: &str) -> String {
    if !text.starts_with('~') {
        return text.to_string();
    }

    let slash_pos = text.find('/');
    let prefix_end = slash_pos.unwrap_or(text.len());
    let prefix = &text[1..prefix_end];
    let suffix = slash_pos.map_or("", |pos| &text[pos..]);

    if prefix.is_empty() {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{suffix}", home.display());
        }
    } else if let Some(home) = get_user_home(prefix) {
        return format!("{home}{suffix}");
    }

    text.to_string()
}

/// Get the home directory for a specific user.
fn get_user_home(username: &str) -> Option<String> {
    if let Ok(current_user) = std::env::var("USER") {
        if username == current_user {
            return dirs::home_dir().map(|p| p.display().to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Special variables
// ---------------------------------------------------------------------------

/// Resolve a special shell variable ($?, $$, $#, $0).
fn resolve_special_variable(ctx: &ExecContext, name: &str) -> Option<String> {
    match name {
        "?" => Some(ctx.last_exit_code.to_string()),
        "$" => Some(std::process::id().to_string()),
        "#" => Some("0".to_string()),
        "0" => Some("oj-shell".to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Word splitting types
// ---------------------------------------------------------------------------

/// A field tracking both IFS-splitting behavior and glob eligibility.
struct ExpandedField {
    text: String,
    glob_eligible: Vec<bool>,
    splittable: bool,
}

impl ExpandedField {
    /// Create a field from literal text (never splittable).
    fn literal(text: String, glob_eligible: bool, process_escapes: bool) -> Self {
        if process_escapes {
            let (text, glob_eligible) = process_glob_escapes(&text);
            Self {
                text,
                glob_eligible,
                splittable: false,
            }
        } else {
            let len = text.len();
            Self {
                text,
                glob_eligible: vec![glob_eligible; len],
                splittable: false,
            }
        }
    }

    /// Create a field from expanded value that should be split on IFS.
    fn splittable(text: String) -> Self {
        let len = text.len();
        Self {
            text,
            glob_eligible: vec![false; len],
            splittable: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Word expansion
// ---------------------------------------------------------------------------

/// Expand a single [`Word`] into a string.
pub(crate) async fn expand_word(ctx: &mut ExecContext, word: &Word) -> Result<String, ExecError> {
    let eligibility = expand_word_with_eligibility(ctx, word).await?;
    Ok(eligibility.text)
}

/// Expand a single [`Word`] into a string with glob eligibility tracking.
async fn expand_word_with_eligibility(
    ctx: &mut ExecContext,
    word: &Word,
) -> Result<GlobEligibility, ExecError> {
    let mut result = GlobEligibility::new();
    let mut is_first_part = true;

    for part in &word.parts {
        match part {
            WordPart::Literal { value, quoted } => match quoted {
                QuoteStyle::Unquoted => {
                    if is_first_part && value.starts_with('~') {
                        let expanded = expand_tilde(value);
                        result.push_eligible_with_escapes(&expanded);
                    } else {
                        result.push_eligible_with_escapes(value);
                    }
                }
                QuoteStyle::Single | QuoteStyle::Double => result.push_ineligible(value),
            },
            WordPart::Variable { name, modifier } => {
                let value = resolve_special_variable(ctx, name)
                    .or_else(|| ctx.variables.get(name.as_str()).cloned());
                let expanded = match modifier {
                    Some(m) => apply_modifier(ctx, name, m, value.as_deref(), word.span).await?,
                    None => match value {
                        Some(v) => v,
                        None => {
                            return Err(ExecError::UndefinedVariable {
                                name: name.clone(),
                                span: word.span,
                            })
                        }
                    },
                };
                result.push_ineligible(&expanded);
            }
            WordPart::CommandSubstitution { body, .. } => {
                let output = super::execute_substitution(ctx, body).await?;
                let trimmed = output.trim_end_matches('\n');
                result.push_ineligible(trimmed);
            }
        }
        is_first_part = false;
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Word splitting expansion
// ---------------------------------------------------------------------------

/// Expand a word with word splitting and glob expansion (POSIX order).
pub(crate) async fn expand_word_split_glob(
    ctx: &mut ExecContext,
    word: &Word,
    config: &GlobConfig,
) -> Result<Vec<String>, ExecError> {
    if is_fully_quoted(word) {
        let result = expand_word(ctx, word).await?;
        return Ok(vec![result]);
    }

    let mut parts: Vec<ExpandedField> = Vec::new();
    let mut is_first_part = true;

    for part in &word.parts {
        match part {
            WordPart::Literal { value, quoted } => {
                let expanded = if is_first_part
                    && matches!(quoted, QuoteStyle::Unquoted)
                    && value.starts_with('~')
                {
                    expand_tilde(value)
                } else {
                    value.clone()
                };

                let is_unquoted = matches!(quoted, QuoteStyle::Unquoted);
                parts.push(ExpandedField::literal(expanded, is_unquoted, is_unquoted));
            }
            WordPart::Variable { name, modifier } => {
                let value = resolve_special_variable(ctx, name)
                    .or_else(|| ctx.variables.get(name.as_str()).cloned());
                let expanded = match modifier {
                    Some(m) => apply_modifier(ctx, name, m, value.as_deref(), word.span).await?,
                    None => match value {
                        Some(v) => v,
                        None => {
                            return Err(ExecError::UndefinedVariable {
                                name: name.clone(),
                                span: word.span,
                            })
                        }
                    },
                };
                parts.push(ExpandedField::splittable(expanded));
            }
            WordPart::CommandSubstitution { body, .. } => {
                let output = super::execute_substitution(ctx, body).await?;
                let trimmed = output.trim_end_matches('\n').to_string();
                parts.push(ExpandedField::splittable(trimmed));
            }
        }
        is_first_part = false;
    }

    let fields = split_fields_with_eligibility(&parts, &get_ifs(ctx));

    let mut result = Vec::new();
    for field in fields {
        if field.has_glob_pattern() {
            let matches = expand_glob_pattern(&field.text, &ctx.cwd, config, word.span)?;
            result.extend(matches);
        } else {
            result.push(field.text);
        }
    }

    Ok(result)
}

/// Check if the word is entirely quoted (single or double).
fn is_fully_quoted(word: &Word) -> bool {
    word.parts.iter().all(|p| match p {
        WordPart::Literal { quoted, .. } => {
            matches!(quoted, QuoteStyle::Single | QuoteStyle::Double)
        }
        WordPart::Variable { .. } | WordPart::CommandSubstitution { .. } => true,
    }) && word.parts.iter().any(|p| {
        matches!(
            p,
            WordPart::Literal {
                quoted: QuoteStyle::Single | QuoteStyle::Double,
                ..
            }
        )
    })
}

/// Get the effective IFS value.
fn get_ifs(ctx: &ExecContext) -> String {
    ctx.variables
        .get("IFS")
        .cloned()
        .unwrap_or_else(|| ctx.ifs.clone())
}

/// Split fields on IFS while preserving glob eligibility.
fn split_fields_with_eligibility(parts: &[ExpandedField], ifs: &str) -> Vec<GlobEligibility> {
    if parts.is_empty() {
        return Vec::new();
    }

    if ifs.is_empty() {
        let mut result = GlobEligibility::new();
        for part in parts {
            for (i, byte) in part.text.bytes().enumerate() {
                let ch = byte as char;
                if part.glob_eligible.get(i).copied().unwrap_or(false) {
                    result.push_eligible(&ch.to_string());
                } else {
                    result.push_ineligible(&ch.to_string());
                }
            }
        }
        return if result.text.is_empty() {
            Vec::new()
        } else {
            vec![result]
        };
    }

    let mut fields: Vec<GlobEligibility> = Vec::new();
    let mut current_field = GlobEligibility::new();
    let mut has_content = false;

    for part in parts {
        if !part.splittable {
            for (i, byte) in part.text.bytes().enumerate() {
                let ch = byte as char;
                if part.glob_eligible.get(i).copied().unwrap_or(false) {
                    current_field.push_eligible(&ch.to_string());
                } else {
                    current_field.push_ineligible(&ch.to_string());
                }
            }
            has_content = true;
        } else {
            if part.text.is_empty() {
                continue;
            }

            let mut chars = part.text.char_indices().peekable();
            while let Some((_, ch)) = chars.next() {
                if ifs.contains(ch) {
                    if !current_field.text.is_empty() || has_content {
                        fields.push(std::mem::take(&mut current_field));
                        has_content = false;
                    }
                    while chars.peek().is_some_and(|(_, c)| ifs.contains(*c)) {
                        chars.next();
                    }
                } else {
                    current_field.push_ineligible(&ch.to_string());
                    has_content = true;
                }
            }
        }
    }

    if !current_field.text.is_empty() || has_content {
        fields.push(current_field);
    }

    fields
}
