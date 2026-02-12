// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Const interpolation and validation.

use crate::parser::ParseError;
use crate::template::escape_for_shell;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

use super::types::{ConstDef, ImportWarning};

/// Regex for `%{ if const.name == "x" }` directives.
///
/// Supports:
/// - `%{ if const.name == "x" }` — equality
/// - `%{ if const.name != "x" }` — inequality
#[allow(clippy::expect_used)]
static IF_DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"%\{~?\s*if\s+const\.([a-zA-Z_][a-zA-Z0-9_]*)\s*(==|!=)\s*"([^"]*)"\s*~?\}"#)
        .expect("constant regex pattern is valid")
});

/// Regex for `%{ else }` directives.
#[allow(clippy::expect_used)]
static ELSE_DIRECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"%\{~?\s*else\s*~?\}").expect("constant regex pattern is valid"));

/// Regex for `%{ endif }` directives.
#[allow(clippy::expect_used)]
static ENDIF_DIRECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"%\{~?\s*endif\s*~?\}").expect("constant regex pattern is valid"));

/// Process `%{ if const.name == "x" }` / `%{ else }` / `%{ endif }` directives.
///
/// Evaluates conditionals based on const values. Supports:
/// - Comparison: `%{ if const.name == "value" }` / `%{ if const.name != "value" }`
/// - `%{ else }` branches and nesting
pub(crate) fn process_const_directives(
    content: &str,
    values: &HashMap<String, String>,
) -> Result<String, String> {
    let mut kept_lines: Vec<&str> = Vec::new();
    // Stack of (active, else_seen) — active means we're emitting lines
    let mut stack: Vec<(bool, bool)> = Vec::new();

    for line in content.split('\n') {
        if let Some(caps) = IF_DIRECTIVE.captures(line) {
            let name = &caps[1];
            let op = &caps[2];
            let literal = &caps[3];
            let value = values.get(name).map(|v| v.as_str()).unwrap_or("");
            let condition = {
                let matches = value == literal;
                if op == "==" {
                    matches
                } else {
                    !matches
                }
            };
            // Only active if parent is active (or we're at top level)
            let parent_active = stack.last().is_none_or(|&(a, _)| a);
            stack.push((parent_active && condition, false));
            continue;
        }

        if ELSE_DIRECTIVE.is_match(line) {
            let len = stack.len();
            if len == 0 {
                return Err("else without matching if".to_string());
            }
            if stack[len - 1].1 {
                return Err("duplicate else".to_string());
            }
            stack[len - 1].1 = true;
            let parent_active = if len > 1 { stack[len - 2].0 } else { true };
            // Flip: if parent is active, toggle current; if parent inactive, stay inactive
            stack[len - 1].0 = parent_active && !stack[len - 1].0;
            continue;
        }

        if ENDIF_DIRECTIVE.is_match(line) {
            if stack.is_empty() {
                return Err("endif without matching if".to_string());
            }
            stack.pop();
            continue;
        }

        // Keep line if all levels are active
        let active = stack.last().is_none_or(|&(a, _)| a);
        if active {
            kept_lines.push(line);
        }
    }

    if !stack.is_empty() {
        return Err("unclosed if directive".to_string());
    }

    Ok(kept_lines.join("\n"))
}

/// Regex for `${raw(const.name)}` patterns.
#[allow(clippy::expect_used)]
static RAW_CONST_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{raw\(const\.([a-zA-Z_][a-zA-Z0-9_]*)\)\}")
        .expect("constant regex pattern is valid")
});

/// Regex for `${const.name}` patterns.
#[allow(clippy::expect_used)]
static CONST_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{const\.([a-zA-Z_][a-zA-Z0-9_]*)\}").expect("constant regex pattern is valid")
});

/// Strip `%{ if/else/endif }` const directives from library content.
///
/// Evaluates all conditionals with empty values, effectively removing all
/// directive lines and any content inside `%{ if const.X != "" }` blocks.
/// Used for lightweight parsing (e.g. extracting entity names) when const
/// values are not yet known.
pub fn strip_const_directives(content: &str) -> Result<String, String> {
    process_const_directives(content, &HashMap::new())
}

/// Interpolate const values into library content.
///
/// 1. Evaluate `%{ if/else/endif }` directives — strip or keep text blocks
/// 2. Substitute `${raw(const.name)}` and `${const.name}` in remaining text
pub fn interpolate_consts(
    content: &str,
    values: &HashMap<String, String>,
) -> Result<String, String> {
    let content = process_const_directives(content, values)?;

    // Replace ${raw(const.name)} with raw values, preserving indentation
    // for multi-line values so they stay aligned inside heredocs.
    let result = replace_raw_consts(&content, values);

    // Replace ${const.name} with shell-escaped values
    Ok(CONST_PATTERN
        .replace_all(&result, |caps: &regex::Captures| {
            let name = &caps[1];
            match values.get(name) {
                Some(val) => escape_for_shell(val),
                None => caps[0].to_string(),
            }
        })
        .to_string())
}

/// Replace `${raw(const.name)}` patterns, indenting multi-line values to match
/// the column where the pattern appears. Without this, lines after the first
/// would lose their alignment inside `<<-` heredocs and get truncated.
fn replace_raw_consts(content: &str, values: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(content.len());
    let mut last_end = 0;

    for caps in RAW_CONST_PATTERN.captures_iter(content) {
        let Some(full_match) = caps.get(0) else {
            continue;
        };
        let name = &caps[1];

        // Append everything before this match
        result.push_str(&content[last_end..full_match.start()]);

        let value = match values.get(name) {
            Some(v) => v.as_str(),
            None => {
                result.push_str(full_match.as_str());
                last_end = full_match.end();
                continue;
            }
        };

        // Find the leading whitespace on the line containing this match
        let line_start = content[..full_match.start()].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let prefix: &str = &content[line_start..full_match.start()];
        let indent: String = prefix.chars().take_while(|c| c.is_whitespace()).collect();

        // For multi-line values, indent continuation lines to match
        if let Some(first_newline) = value.find('\n') {
            result.push_str(&value[..first_newline]);
            for line in value[first_newline..].split('\n').skip(1) {
                result.push('\n');
                if !line.is_empty() {
                    result.push_str(&indent);
                }
                result.push_str(line);
            }
        } else {
            result.push_str(value);
        }

        last_end = full_match.end();
    }

    result.push_str(&content[last_end..]);
    result
}

/// Validate const values against const definitions.
///
/// Returns the resolved values map (with defaults applied) and any warnings.
pub fn validate_consts(
    defs: &HashMap<String, ConstDef>,
    provided: &HashMap<String, String>,
    source: &str,
) -> Result<(HashMap<String, String>, Vec<ImportWarning>), ParseError> {
    let mut values = HashMap::new();
    let mut warnings = Vec::new();

    // Check each defined const
    for (name, def) in defs {
        match provided.get(name) {
            Some(val) => {
                values.insert(name.clone(), val.clone());
            }
            None => match &def.default {
                Some(default) => {
                    values.insert(name.clone(), default.clone());
                }
                None => {
                    return Err(ParseError::InvalidFormat {
                        location: format!("import \"{}\"", source),
                        message: format!(
                            "missing required const '{}'; add const \"{}\" {{ value = \"...\" }}",
                            name, name
                        ),
                    });
                }
            },
        }
    }

    // Warn on unknown consts
    for name in provided.keys() {
        if !defs.contains_key(name) {
            warnings.push(ImportWarning::UnknownConst {
                source: source.to_string(),
                name: name.clone(),
            });
        }
    }

    Ok((values, warnings))
}
