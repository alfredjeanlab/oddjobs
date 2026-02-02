// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Glob (pathname) expansion for shell words.
//!
//! Implements POSIX-style glob expansion where unquoted metacharacters (`*`, `?`, `[...]`)
//! are expanded against the filesystem. Quoted strings suppress glob expansion.

use std::path::Path;

use crate::Span;

use super::error::ExecError;

/// Configuration for glob expansion behavior.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GlobConfig {
    /// If true, return empty vec when no matches (like bash nullglob).
    /// If false, return literal pattern when no matches (default POSIX).
    pub nullglob: bool,
}

/// Track which character positions in an expanded string are glob-eligible.
///
/// When a word contains a mix of quoted and unquoted parts, or variables and literals,
/// we need to track which characters came from unquoted literals so we know which
/// glob metacharacters should actually trigger expansion.
#[derive(Debug, Default)]
pub(crate) struct GlobEligibility {
    /// The assembled text from all word parts.
    pub text: String,
    /// True at index `i` if the byte at position `i` can trigger glob expansion.
    eligible: Vec<bool>,
}

impl GlobEligibility {
    /// Create a new empty eligibility tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append text that IS eligible for glob expansion (from unquoted literals).
    pub fn push_eligible(&mut self, s: &str) {
        self.text.push_str(s);
        self.eligible.extend(std::iter::repeat_n(true, s.len()));
    }

    /// Append text that is NOT eligible for glob expansion (from quoted literals or variables).
    pub fn push_ineligible(&mut self, s: &str) {
        self.text.push_str(s);
        self.eligible.extend(std::iter::repeat_n(false, s.len()));
    }

    /// Append text with backslash-escaped glob metacharacters processed.
    ///
    /// For unquoted literals that may contain backslash escapes:
    /// - `\*`, `\?`, `\[` become literal characters (ineligible for glob)
    /// - `\\` becomes a single `\` (ineligible for glob)
    /// - All other characters are eligible for glob expansion
    pub fn push_eligible_with_escapes(&mut self, s: &str) {
        let (text, eligible) = process_glob_escapes(s);
        self.text.push_str(&text);
        self.eligible.extend(eligible);
    }

    /// Check if the text contains any glob-eligible metacharacters.
    pub fn has_glob_pattern(&self) -> bool {
        self.text.bytes().enumerate().any(|(i, b)| {
            self.eligible.get(i).copied().unwrap_or(false) && matches!(b, b'*' | b'?' | b'[')
        })
    }
}

/// Process backslash escapes in unquoted text for glob eligibility.
///
/// Returns (processed_text, per-char eligibility). Escaped glob metacharacters
/// (`\*`, `\?`, `\[`, `\\`) become literal and ineligible; all else is eligible.
pub(crate) fn process_glob_escapes(s: &str) -> (String, Vec<bool>) {
    let mut text = String::new();
    let mut eligible = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if matches!(next, '*' | '?' | '[' | '\\') {
                    chars.next();
                    text.push(next);
                    eligible.push(false);
                    continue;
                }
            }
        }
        text.push(ch);
        eligible.push(true);
    }

    (text, eligible)
}

/// Check if a pattern's filename component starts with a dot.
///
/// This is used to determine if hidden files should be included in matches.
fn pattern_matches_hidden(pattern: &str) -> bool {
    // Get the filename portion of the pattern
    Path::new(pattern)
        .file_name()
        .and_then(|f| f.to_str())
        .map(|f| f.starts_with('.'))
        .unwrap_or(false)
}

/// Expand a glob pattern against the filesystem.
///
/// # Arguments
///
/// * `pattern` - The glob pattern to expand (may be relative or absolute)
/// * `cwd` - The current working directory for resolving relative patterns
/// * `config` - Glob expansion configuration
/// * `span` - Source span for error reporting
///
/// # Returns
///
/// A vector of matching paths as strings. If no matches are found:
/// - With `nullglob: true`: returns an empty vector
/// - With `nullglob: false` (POSIX default): returns the original pattern
pub(crate) fn expand_glob_pattern(
    pattern: &str,
    cwd: &Path,
    config: &GlobConfig,
    span: Span,
) -> Result<Vec<String>, ExecError> {
    // Construct the full pattern path for matching
    let full_pattern = if Path::new(pattern).is_absolute() {
        pattern.to_string()
    } else {
        cwd.join(pattern).to_string_lossy().into_owned()
    };

    // Determine if we should include hidden files based on the pattern.
    // POSIX: patterns starting with . match hidden files, others don't.
    let include_hidden = pattern_matches_hidden(pattern);

    // Compile and execute the glob pattern
    let paths = glob::glob(&full_pattern).map_err(|e| ExecError::GlobPattern {
        pattern: pattern.to_string(),
        message: e.msg.to_string(),
        span,
    })?;

    // Collect matching paths, converting back to relative strings
    let mut matches: Vec<String> = paths
        .filter_map(|result| {
            // Skip paths that had errors (e.g., permission denied)
            result.ok()
        })
        .filter_map(|path| {
            // Get the relative path string
            let relative = if let Ok(rel) = path.strip_prefix(cwd) {
                rel.to_string_lossy().into_owned()
            } else {
                path.to_string_lossy().into_owned()
            };

            // Filter hidden files unless the pattern explicitly matches them
            if !include_hidden {
                // Check if the filename starts with a dot
                if let Some(filename) = Path::new(&relative).file_name() {
                    if filename.to_string_lossy().starts_with('.') {
                        return None;
                    }
                }
            }

            Some(relative)
        })
        .collect();

    // Sort results lexicographically (POSIX requirement)
    matches.sort();

    // Handle no-match case per configuration
    if matches.is_empty() && !config.nullglob {
        Ok(vec![pattern.to_string()])
    } else {
        Ok(matches)
    }
}

#[cfg(test)]
#[path = "expand_glob_tests.rs"]
mod tests;
