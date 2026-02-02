// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runbook file discovery

use crate::parser::Format;
use crate::{parse_runbook_with_format, Runbook};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors from runbook file scanning
#[derive(Debug, Error)]
pub enum FindError {
    #[error("'{0}' defined in multiple runbooks; use --runbook <file>")]
    Duplicate(String),
    #[error("{name} not found; {count} runbook(s) skipped due to errors:\n{details}")]
    NotFoundSkipped {
        name: String,
        count: usize,
        details: String,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Scan `.oj/runbooks/` recursively for the file defining command `name`.
pub fn find_runbook_by_command(
    runbook_dir: &Path,
    name: &str,
) -> Result<Option<Runbook>, FindError> {
    find_runbook(runbook_dir, name, |rb| rb.get_command(name).is_some())
}

/// Scan `.oj/runbooks/` recursively for the file defining worker `name`.
pub fn find_runbook_by_worker(
    runbook_dir: &Path,
    name: &str,
) -> Result<Option<Runbook>, FindError> {
    find_runbook(runbook_dir, name, |rb| rb.get_worker(name).is_some())
}

/// Scan `.oj/runbooks/` recursively for the file defining queue `name`.
pub fn find_runbook_by_queue(runbook_dir: &Path, name: &str) -> Result<Option<Runbook>, FindError> {
    find_runbook(runbook_dir, name, |rb| rb.get_queue(name).is_some())
}

/// Scan `.oj/runbooks/` and collect all command definitions.
/// Returns a sorted vec of (command_name, CommandDef) pairs.
/// Skips runbooks that fail to parse (logs warnings).
pub fn collect_all_commands(
    runbook_dir: &Path,
) -> Result<Vec<(String, crate::CommandDef)>, FindError> {
    if !runbook_dir.exists() {
        return Ok(Vec::new());
    }
    let files = collect_runbook_files(runbook_dir)?;
    let mut commands = Vec::new();
    for (path, format) in files {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping unreadable runbook");
                continue;
            }
        };
        let runbook = match parse_runbook_with_format(&content, format) {
            Ok(rb) => rb,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping invalid runbook");
                continue;
            }
        };
        for (name, cmd) in runbook.commands {
            commands.push((name, cmd));
        }
    }
    commands.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(commands)
}

fn find_runbook(
    runbook_dir: &Path,
    name: &str,
    matches: impl Fn(&Runbook) -> bool,
) -> Result<Option<Runbook>, FindError> {
    if !runbook_dir.exists() {
        return Ok(None);
    }
    let files = collect_runbook_files(runbook_dir)?;
    let mut found: Option<Runbook> = None;
    let mut skipped: Vec<(PathBuf, String)> = Vec::new();
    for (path, format) in files {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping unreadable runbook");
                skipped.push((path, e.to_string()));
                continue;
            }
        };
        let runbook = match parse_runbook_with_format(&content, format) {
            Ok(rb) => rb,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping invalid runbook");
                skipped.push((path, e.to_string()));
                continue;
            }
        };
        if matches(&runbook) {
            if found.is_some() {
                return Err(FindError::Duplicate(name.to_string()));
            }
            found = Some(runbook);
        }
    }
    if found.is_none() && !skipped.is_empty() {
        let details = skipped
            .iter()
            .map(|(p, e)| format!("  {}: {e}", p.display()))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(FindError::NotFoundSkipped {
            name: name.to_string(),
            count: skipped.len(),
            details,
        });
    }
    Ok(found)
}

/// Recursively collect all runbook files (`.hcl`, `.toml`, `.json`) under `dir`.
fn collect_runbook_files(dir: &Path) -> Result<Vec<(PathBuf, Format)>, std::io::Error> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(format) = format_for_path(&path) {
                files.push((path, format));
            }
        }
    }
    Ok(files)
}

fn format_for_path(path: &Path) -> Option<Format> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("toml") => Some(Format::Toml),
        Some("hcl") => Some(Format::Hcl),
        Some("json") => Some(Format::Json),
        _ => None,
    }
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
