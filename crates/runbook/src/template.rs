// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Template variable interpolation

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Regex pattern for ${variable_name} or ${namespace.variable_name}
// Allow expect here as the regex is compile-time verified to be valid
#[allow(clippy::expect_used)]
pub static VAR_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*(?:\.[a-zA-Z_][a-zA-Z0-9_-]*)*)\}")
        .expect("constant regex pattern is valid")
});

// Regex pattern for ${VAR:-default} environment variable expansion
#[allow(clippy::expect_used)]
static ENV_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{(\w+):-([^}]*)\}").expect("constant regex pattern is valid"));

/// Escape a string for safe use inside shell single-quoted contexts.
///
/// Single quotes in the value are replaced with `'\''` (end quote, escaped
/// literal quote, start quote), which is the standard bash idiom for
/// embedding a single quote inside a single-quoted string.
///
/// This escaping is also safe in unquoted shell contexts, though values with
/// spaces will still be word-split. Runbook authors should wrap `${var}`
/// references in single quotes for values that may contain spaces.
pub fn escape_for_shell(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Interpolate `${name}` placeholders with values from the vars map
///
/// Also expands `${VAR:-default}` patterns from environment variables.
/// Environment variables are expanded first, then template variables.
///
/// Unknown template variables are left as-is.
pub fn interpolate(template: &str, vars: &HashMap<String, String>) -> String {
    interpolate_inner(template, vars, false)
}

/// Interpolate `${name}` placeholders with shell-safe escaping.
///
/// Like [`interpolate`], but escapes substituted values for safe use in
/// shell contexts (single quotes are escaped using the `'\''` idiom).
/// Use this for shell commands; use [`interpolate`] for prompts and
/// other non-shell contexts.
pub fn interpolate_shell(template: &str, vars: &HashMap<String, String>) -> String {
    interpolate_inner(template, vars, true)
}

fn interpolate_inner(template: &str, vars: &HashMap<String, String>, shell_escape: bool) -> String {
    // First expand ${VAR:-default} patterns from environment
    let result = ENV_PATTERN
        .replace_all(template, |caps: &regex::Captures| {
            let var_name = &caps[1];
            let default_value = &caps[2];
            std::env::var(var_name).unwrap_or_else(|_| default_value.to_string())
        })
        .to_string();

    // Then expand ${var} or ${namespace.var} patterns from provided vars
    VAR_PATTERN
        .replace_all(&result, |caps: &regex::Captures| {
            let name = &caps[1];
            match vars.get(name) {
                Some(val) if shell_escape => escape_for_shell(val),
                Some(val) => val.clone(),
                None => caps[0].to_string(),
            }
        })
        .to_string()
}

#[cfg(test)]
#[path = "template_tests.rs"]
mod tests;
