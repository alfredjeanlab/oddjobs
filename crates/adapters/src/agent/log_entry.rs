// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Structured log entry extraction from Claude's JSONL session log.
//!
//! Parses Claude Code's session log format into human-readable `AgentLogEntry`
//! values. The parser is incremental: it takes a byte offset and returns new
//! entries plus the updated offset, so the watcher can resume from where it
//! left off.

use oj_core::AgentId;
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// Message carrying agent log entries over a channel.
///
/// Tuple of (agent_id, entries) sent from watcher to agent logger.
pub type AgentLogMessage = (AgentId, Vec<AgentLogEntry>);

/// A structured entry extracted from Claude's JSONL session log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLogEntry {
    pub timestamp: String,
    pub kind: EntryKind,
}

/// The kind of activity recorded in a log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// File read via Read tool
    FileRead { path: String },
    /// File written via Write tool
    FileWrite {
        path: String,
        new: bool,
        lines: usize,
    },
    /// File edited via Edit tool
    FileEdit { path: String },
    /// Notebook edited via NotebookEdit tool
    NotebookEdit { path: String },
    /// Bash command executed
    BashCommand {
        command: String,
        exit_code: Option<i32>,
    },
    /// oj CLI call (oj run, oj emit, etc.)
    OjCall { args: Vec<String> },
    /// Turn completed (assistant message finished)
    TurnComplete {
        duration_secs: Option<u64>,
        tokens: Option<u64>,
    },
    /// Error encountered
    Error { message: String },
}

impl fmt::Display for AgentLogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.timestamp, self.kind)
    }
}

impl fmt::Display for EntryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntryKind::FileRead { path } => write!(f, "read: {}", path),
            EntryKind::FileWrite { path, new, lines } => {
                if *new {
                    write!(f, "wrote: {} (new, {} lines)", path, lines)
                } else {
                    write!(f, "wrote: {} ({} lines)", path, lines)
                }
            }
            EntryKind::FileEdit { path } => write!(f, "edited: {}", path),
            EntryKind::NotebookEdit { path } => write!(f, "edited: {} (notebook)", path),
            EntryKind::BashCommand { command, exit_code } => match exit_code {
                Some(code) => write!(f, "bash: {} (exit {})", command, code),
                None => write!(f, "bash: {}", command),
            },
            EntryKind::OjCall { args } => write!(f, "oj: {}", args.join(" ")),
            EntryKind::TurnComplete {
                duration_secs,
                tokens,
            } => {
                let dur = duration_secs
                    .map(|s| format!("{}s", s))
                    .unwrap_or_else(|| "?s".to_string());
                let tok = tokens
                    .map(format_tokens)
                    .unwrap_or_else(|| "? tokens".to_string());
                write!(f, "turn complete ({}, {})", dur, tok)
            }
            EntryKind::Error { message } => write!(f, "error: {}", message),
        }
    }
}

/// Format token count in human-readable form (e.g., "1.5k tokens").
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1000 {
        let k = tokens as f64 / 1000.0;
        if k == k.floor() {
            format!("{}k tokens", k as u64)
        } else {
            format!("{:.1}k tokens", k)
        }
    } else {
        format!("{} tokens", tokens)
    }
}

// --- JSON Extraction Helpers ---

/// Extract a string value from a JSON object by key.
fn get_str<'a>(obj: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(|v| v.as_str())
}

/// Extract a string value from a JSON object, returning a default if missing.
fn get_str_or<'a>(obj: &'a serde_json::Value, key: &str, default: &'a str) -> &'a str {
    get_str(obj, key).unwrap_or(default)
}

// --- Entry Construction Helper ---

/// Push a new log entry with the given timestamp and kind.
fn push_entry(entries: &mut Vec<AgentLogEntry>, timestamp: String, kind: EntryKind) {
    entries.push(AgentLogEntry { timestamp, kind });
}

/// Parse new log entries from a JSONL session log starting at the given byte offset.
///
/// Returns the extracted entries and the new byte offset. The caller should
/// persist the offset and pass it back on the next call to avoid re-processing.
pub fn parse_entries_from(path: &Path, offset: u64) -> (Vec<AgentLogEntry>, u64) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (Vec::new(), offset),
    };

    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::Start(offset)).is_err() {
        return (Vec::new(), offset);
    }

    let mut entries = Vec::new();
    let mut current_offset = offset;
    let mut last_user_timestamp: Option<String> = None;

    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(n) => {
                // Only process complete lines (ending with newline)
                if !line.ends_with('\n') {
                    // Incomplete line — don't advance offset, will re-read next time
                    break;
                }
                current_offset += n as u64;

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let json: serde_json::Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                extract_entries(&json, &mut entries, &mut last_user_timestamp);
            }
            Err(_) => break,
        }
    }

    (entries, current_offset)
}

/// Extract log entries from a single JSONL record.
fn extract_entries(
    json: &serde_json::Value,
    entries: &mut Vec<AgentLogEntry>,
    last_user_timestamp: &mut Option<String>,
) {
    let record_type = get_str_or(json, "type", "");

    // Track user message timestamps for turn duration calculation
    if record_type == "user" {
        if let Some(ts) = extract_timestamp(json) {
            *last_user_timestamp = Some(ts);
        }
        return;
    }

    // Check for error indicators
    if let Some(error_msg) = extract_error(json) {
        let timestamp = extract_timestamp(json).unwrap_or_default();
        push_entry(entries, timestamp, EntryKind::Error { message: error_msg });
        return;
    }

    if record_type == "assistant" {
        let message = match json.get("message") {
            Some(m) => m,
            None => return,
        };

        // Extract tool_use blocks from content
        if let Some(content) = message.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if get_str(block, "type") != Some("tool_use") {
                    continue;
                }

                let tool_name = match get_str(block, "name") {
                    Some(name) => name,
                    None => continue,
                };

                let input = block
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let timestamp = extract_timestamp(json).unwrap_or_default();

                match tool_name {
                    "Read" => {
                        if let Some(path) = get_str(&input, "file_path") {
                            push_entry(
                                entries,
                                timestamp,
                                EntryKind::FileRead {
                                    path: path.to_string(),
                                },
                            );
                        }
                    }
                    "Write" => {
                        if let Some(path) = get_str(&input, "file_path") {
                            let lines = get_str_or(&input, "content", "").lines().count();
                            push_entry(
                                entries,
                                timestamp,
                                EntryKind::FileWrite {
                                    path: path.to_string(),
                                    new: true, // We can't know from tool_use alone; mark as new
                                    lines,
                                },
                            );
                        }
                    }
                    "Edit" => {
                        if let Some(path) = get_str(&input, "file_path") {
                            push_entry(
                                entries,
                                timestamp,
                                EntryKind::FileEdit {
                                    path: path.to_string(),
                                },
                            );
                        }
                    }
                    "NotebookEdit" => {
                        if let Some(path) = get_str(&input, "notebook_path") {
                            push_entry(
                                entries,
                                timestamp,
                                EntryKind::NotebookEdit {
                                    path: path.to_string(),
                                },
                            );
                        }
                    }
                    "Bash" => {
                        let command = get_str_or(&input, "command", "").to_string();
                        let trimmed_cmd = command.trim();

                        // Check if this is an oj CLI call
                        if trimmed_cmd == "oj"
                            || trimmed_cmd.starts_with("oj ")
                            || trimmed_cmd.starts_with("./oj ")
                        {
                            let args: Vec<String> =
                                if let Some(rest) = trimmed_cmd.strip_prefix("./oj ") {
                                    rest.split_whitespace().map(String::from).collect()
                                } else if let Some(rest) = trimmed_cmd.strip_prefix("oj ") {
                                    rest.split_whitespace().map(String::from).collect()
                                } else {
                                    Vec::new()
                                };
                            push_entry(entries, timestamp, EntryKind::OjCall { args });
                        } else {
                            // Truncate long commands for display
                            let display_cmd = if command.len() > 80 {
                                format!("{}...", &command[..77])
                            } else {
                                command
                            };
                            push_entry(
                                entries,
                                timestamp,
                                EntryKind::BashCommand {
                                    command: display_cmd,
                                    exit_code: None,
                                },
                            );
                        }
                    }
                    _ => {} // Skip unknown tools
                }
            }
        }

        // Check for turn completion (end_turn stop reason)
        if get_str_or(message, "stop_reason", "") == "end_turn" {
            let timestamp = extract_timestamp(json).unwrap_or_default();
            let tokens = message
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|t| t.as_u64());

            // Compute duration from last user message
            let duration_secs =
                compute_duration_secs(last_user_timestamp.as_deref(), timestamp.as_str());

            push_entry(
                entries,
                timestamp,
                EntryKind::TurnComplete {
                    duration_secs,
                    tokens,
                },
            );
        }
    }

    // Handle tool results — attach exit codes to BashCommand entries
    if record_type == "result" {
        if let Some(content) = get_str(json, "content") {
            // Look for exit code patterns in result content
            if let Some(exit_code) = extract_exit_code(content) {
                // Try to update the most recent BashCommand entry
                for entry in entries.iter_mut().rev() {
                    if let EntryKind::BashCommand {
                        exit_code: ref mut ec,
                        ..
                    } = entry.kind
                    {
                        if ec.is_none() {
                            *ec = Some(exit_code);
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Extract a timestamp from a JSONL record.
///
/// Looks for `timestamp` or `message.created_at` fields.
fn extract_timestamp(json: &serde_json::Value) -> Option<String> {
    // Direct timestamp field
    if let Some(ts) = get_str(json, "timestamp") {
        return Some(ts.to_string());
    }

    // Message-level timestamp (ISO 8601 from Claude API)
    if let Some(message) = json.get("message") {
        if let Some(ts) = get_str(message, "created_at") {
            return Some(ts.to_string());
        }
    }

    // Epoch-based timestamp (Claude Code uses costMicros or similar)
    if let Some(ts) = get_str(json, "isoTimestamp") {
        return Some(ts.to_string());
    }

    None
}

/// Extract error message from a JSONL record.
fn extract_error(json: &serde_json::Value) -> Option<String> {
    // Direct error field
    if let Some(err) = get_str(json, "error") {
        return Some(err.to_string());
    }

    // Nested in message
    if let Some(message) = json.get("message") {
        if let Some(err) = get_str(message, "error") {
            return Some(err.to_string());
        }
    }

    None
}

/// Compute duration between two ISO 8601 timestamps in seconds.
///
/// Returns None if either timestamp is missing or unparseable.
fn compute_duration_secs(start: Option<&str>, end: &str) -> Option<u64> {
    let start = start?;
    // Simple ISO 8601 parsing — extract epoch seconds
    let start_secs = parse_iso_epoch(start)?;
    let end_secs = parse_iso_epoch(end)?;
    Some(end_secs.saturating_sub(start_secs))
}

/// Parse a subset of ISO 8601 timestamps to epoch seconds.
///
/// Handles formats like "2026-01-30T08:17:05Z" and "2026-01-30T08:17:05.123Z".
fn parse_iso_epoch(s: &str) -> Option<u64> {
    // Minimal parser for YYYY-MM-DDTHH:MM:SS[.fff]Z
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }

    let year: i64 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let hour: u64 = s[11..13].parse().ok()?;
    let min: u64 = s[14..16].parse().ok()?;
    let sec: u64 = s[17..19].parse().ok()?;

    // Convert date to days since epoch (Howard Hinnant's algorithm, inverse)
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = month;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as u64 * 146097 + doe - 719468;

    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

/// Extract exit code from tool result content.
fn extract_exit_code(content: &str) -> Option<i32> {
    // Claude Code tool results sometimes include exit code info
    // Look for patterns like "exit code: 0" or "Exit code: 1"
    let lower = content.to_lowercase();
    if let Some(pos) = lower.find("exit code:") {
        let after = &content[pos + 10..];
        let code_str: String = after
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        return code_str.parse().ok();
    }
    None
}

#[cfg(test)]
#[path = "log_entry_tests.rs"]
mod tests;
