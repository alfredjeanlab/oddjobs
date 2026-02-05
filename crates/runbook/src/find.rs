// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runbook file discovery

use crate::parser::Format;
use crate::{parse_runbook_with_format, CommandDef, Runbook};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Leading comment block extracted from a runbook file.
pub struct FileComment {
    /// Text up to the first blank comment line (short description).
    pub short: String,
    /// Remaining comment text after the blank line.
    pub long: String,
}

/// Extract the leading comment block from a runbook file's raw content.
///
/// Reads lines starting with `#`, strips the `# ` prefix, and returns:
/// - `short`: text up to the first blank comment line (two consecutive newlines)
/// - `long`: remaining comment text after the blank line
///
/// Returns `None` if the file has no leading comment block.
pub fn extract_file_comment(content: &str) -> Option<FileComment> {
    let mut lines = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let text = trimmed
                .strip_prefix("# ")
                .unwrap_or(trimmed.strip_prefix('#').unwrap_or(""));
            lines.push(text.to_string());
        } else if trimmed.is_empty() && lines.is_empty() {
            continue;
        } else {
            break;
        }
    }

    if lines.is_empty() {
        return None;
    }

    let split_pos = lines.iter().position(|l| l.is_empty());
    let (short_lines, long_lines) = match split_pos {
        Some(pos) => (&lines[..pos], &lines[pos + 1..]),
        None => (lines.as_slice(), &[][..]),
    };

    Some(FileComment {
        short: short_lines.join("\n"),
        long: long_lines.join("\n"),
    })
}

/// Extract comment blocks preceding each `command "name"` block in HCL content.
///
/// Scans the raw text (not the HCL AST) for lines matching `command "name" {`
/// and collects the preceding `#`-comment block for each.
///
/// Returns a map of command_name → FileComment.
pub fn extract_block_comments(content: &str) -> HashMap<String, FileComment> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = HashMap::new();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Match: command "name" { (with optional trailing content)
        let Some(rest) = trimmed.strip_prefix("command ") else {
            continue;
        };
        let rest = rest.trim();
        let name = match rest.strip_prefix('"') {
            Some(after_quote) => match after_quote.find('"') {
                Some(end) => &after_quote[..end],
                None => continue,
            },
            None => continue,
        };

        // Walk backwards from line i-1 collecting # comment lines.
        // Stop at a non-comment, non-blank line or at the start of file.
        let mut comment_lines = Vec::new();
        let mut j = i;
        while j > 0 {
            j -= 1;
            let prev = lines[j].trim();
            if prev.starts_with('#') {
                let text = prev
                    .strip_prefix("# ")
                    .unwrap_or(prev.strip_prefix('#').unwrap_or(""));
                comment_lines.push(text);
            } else if prev.is_empty() {
                if comment_lines.is_empty() {
                    continue; // skip blanks between block and comment
                } else {
                    break; // blank line above the comment block = stop
                }
            } else {
                break; // hit a non-comment line (e.g., closing brace of previous block)
            }
        }
        comment_lines.reverse();

        if comment_lines.is_empty() {
            continue;
        }

        // Split into short/long on first blank comment line
        let owned: Vec<String> = comment_lines.iter().map(|s| s.to_string()).collect();
        let split_pos = owned.iter().position(|l| l.is_empty());
        let (short_lines, long_lines) = match split_pos {
            Some(pos) => (&owned[..pos], &owned[pos + 1..]),
            None => (owned.as_slice(), &[][..]),
        };

        result.insert(
            name.to_string(),
            FileComment {
                short: short_lines.join("\n"),
                long: long_lines.join("\n"),
            },
        );
    }

    result
}

/// Find a command definition and its runbook file comment.
pub fn find_command_with_comment(
    runbook_dir: &Path,
    command_name: &str,
) -> Result<Option<(CommandDef, Option<FileComment>)>, FindError> {
    if !runbook_dir.exists() {
        return Ok(None);
    }
    let files = collect_runbook_files(runbook_dir)?;
    for (path, format) in files {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let runbook = match parse_runbook_with_format(&content, format) {
            Ok(rb) => rb,
            Err(_) => continue,
        };
        if let Some(cmd) = runbook.commands.get(command_name) {
            let mut block_comments = extract_block_comments(&content);
            let comment = block_comments
                .remove(command_name)
                .or_else(|| extract_file_comment(&content));
            return Ok(Some((cmd.clone(), comment)));
        }
    }
    Ok(None)
}

/// Errors from runbook file scanning
#[derive(Debug, Error)]
pub enum FindError {
    #[error("'{0}' defined in multiple runbooks; use --runbook <file>")]
    Duplicate(String),
    #[error("{entity_type} '{name}' defined in both {} and {}", file_a.display(), file_b.display())]
    DuplicateAcrossFiles {
        entity_type: String,
        name: String,
        file_a: PathBuf,
        file_b: PathBuf,
    },
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

/// Scan `.oj/runbooks/` recursively for the file defining cron `name`.
pub fn find_runbook_by_cron(runbook_dir: &Path, name: &str) -> Result<Option<Runbook>, FindError> {
    find_runbook(runbook_dir, name, |rb| rb.get_cron(name).is_some())
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
        let block_comments = extract_block_comments(&content);
        let file_comment = extract_file_comment(&content);
        for (name, mut cmd) in runbook.commands {
            if cmd.description.is_none() {
                let comment = block_comments.get(&name).or(file_comment.as_ref());
                if let Some(comment) = comment {
                    let desc_line = comment
                        .short
                        .lines()
                        .nth(1)
                        .or_else(|| comment.short.lines().next())
                        .unwrap_or("");
                    if !desc_line.is_empty() {
                        cmd.description = Some(desc_line.to_string());
                    }
                }
            }
            commands.push((name, cmd));
        }
    }
    commands.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(commands)
}

/// Scan `.oj/runbooks/` and collect all queue definitions.
/// Returns a sorted vec of (queue_name, QueueDef) pairs.
/// Skips runbooks that fail to parse (logs warnings).
pub fn collect_all_queues(runbook_dir: &Path) -> Result<Vec<(String, crate::QueueDef)>, FindError> {
    if !runbook_dir.exists() {
        return Ok(Vec::new());
    }
    let files = collect_runbook_files(runbook_dir)?;
    let mut queues = Vec::new();
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
        for (name, queue) in runbook.queues {
            queues.push((name, queue));
        }
    }
    queues.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(queues)
}

/// Scan `.oj/runbooks/` and collect all worker definitions.
/// Returns a sorted vec of (worker_name, WorkerDef) pairs.
/// Skips runbooks that fail to parse (logs warnings).
pub fn collect_all_workers(
    runbook_dir: &Path,
) -> Result<Vec<(String, crate::WorkerDef)>, FindError> {
    if !runbook_dir.exists() {
        return Ok(Vec::new());
    }
    let files = collect_runbook_files(runbook_dir)?;
    let mut workers = Vec::new();
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
        for (name, worker) in runbook.workers {
            workers.push((name, worker));
        }
    }
    workers.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(workers)
}

/// Scan `.oj/runbooks/` and collect all cron definitions.
/// Returns a sorted vec of (cron_name, CronDef) pairs.
/// Skips runbooks that fail to parse (logs warnings).
pub fn collect_all_crons(runbook_dir: &Path) -> Result<Vec<(String, crate::CronDef)>, FindError> {
    if !runbook_dir.exists() {
        return Ok(Vec::new());
    }
    let files = collect_runbook_files(runbook_dir)?;
    let mut crons = Vec::new();
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
        for (name, cron) in runbook.crons {
            crons.push((name, cron));
        }
    }
    crons.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(crons)
}

/// Validate all runbooks in a directory for cross-file conflicts.
///
/// Returns errors for any entity name defined in multiple files within the same
/// entity type (commands, jobs, agents, queues, workers).
pub fn validate_runbook_dir(runbook_dir: &Path) -> Result<(), Vec<FindError>> {
    if !runbook_dir.exists() {
        return Ok(());
    }
    let files = collect_runbook_files(runbook_dir).map_err(|e| vec![FindError::Io(e)])?;

    // Track (entity_type, name) -> source file path
    let mut seen: HashMap<(&str, String), PathBuf> = HashMap::new();
    let mut errors = Vec::new();

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

        for entity_type_names in [
            (
                "command",
                runbook.commands.keys().cloned().collect::<Vec<_>>(),
            ),
            (
                "job",
                runbook.jobs.keys().cloned().collect::<Vec<_>>(),
            ),
            ("agent", runbook.agents.keys().cloned().collect::<Vec<_>>()),
            ("queue", runbook.queues.keys().cloned().collect::<Vec<_>>()),
            (
                "worker",
                runbook.workers.keys().cloned().collect::<Vec<_>>(),
            ),
            ("cron", runbook.crons.keys().cloned().collect::<Vec<_>>()),
        ] {
            let (entity_type, names) = entity_type_names;
            for name in names {
                let key = (entity_type, name.clone());
                if let Some(prev_path) = seen.get(&key) {
                    errors.push(FindError::DuplicateAcrossFiles {
                        entity_type: entity_type.to_string(),
                        name,
                        file_a: prev_path.clone(),
                        file_b: path.clone(),
                    });
                } else {
                    seen.insert(key, path.clone());
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Check all runbook files for parse errors, returning human-readable warnings.
///
/// Returns one warning string per file that failed to parse. An empty vec
/// means all files parsed successfully (or no files exist).
pub fn runbook_parse_warnings(runbook_dir: &Path) -> Vec<String> {
    let files = match collect_runbook_files(runbook_dir) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut warnings = Vec::new();
    for (path, format) in files {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        if let Err(e) = parse_runbook_with_format(&content, format) {
            warnings.push(format!("{}: {e}", path.display()));
        }
    }
    warnings
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
    files.sort_by(|(a, _), (b, _)| a.cmp(b));
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
