// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Structured log entry extraction from Claude's JSONL session log.

use oj_core::AgentId;
use std::fmt;

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
    FileWrite { path: String, new: bool, lines: usize },
    /// File edited via Edit tool
    FileEdit { path: String },
    /// Notebook edited via NotebookEdit tool
    NotebookEdit { path: String },
    /// Bash command executed
    BashCommand { command: String, exit_code: Option<i32> },
    /// oj CLI call (oj run, oj emit, etc.)
    OjCall { args: Vec<String> },
    /// Turn completed (assistant message finished)
    TurnComplete { duration_secs: Option<u64>, tokens: Option<u64> },
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
                let tag = if *new { "new, " } else { "" };
                write!(f, "wrote: {path} ({tag}{lines} lines)")
            }
            EntryKind::FileEdit { path } => write!(f, "edited: {}", path),
            EntryKind::NotebookEdit { path } => write!(f, "edited: {} (notebook)", path),
            EntryKind::BashCommand { command, exit_code } => match exit_code {
                Some(code) => write!(f, "bash: {} (exit {})", command, code),
                None => write!(f, "bash: {}", command),
            },
            EntryKind::OjCall { args } => write!(f, "oj: {}", args.join(" ")),
            EntryKind::TurnComplete { duration_secs, tokens } => {
                let dur = duration_secs.map(|s| format!("{s}s")).unwrap_or_else(|| "?s".into());
                let tok = tokens.map(format_tokens).unwrap_or_else(|| "? tokens".into());
                write!(f, "turn complete ({dur}, {tok})")
            }
            EntryKind::Error { message } => write!(f, "error: {}", message),
        }
    }
}

/// Format token count in human-readable form (e.g., "1.5k tokens").
fn format_tokens(tokens: u64) -> String {
    if tokens < 1000 {
        return format!("{tokens} tokens");
    }
    let k = tokens as f64 / 1000.0;
    if k == k.floor() {
        format!("{}k tokens", k as u64)
    } else {
        format!("{k:.1}k tokens")
    }
}

/// Extract a string value from a JSON object by key.
fn get_str<'a>(obj: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(|v| v.as_str())
}

/// Extract log entries from a single JSONL record.
pub fn extract_entries(
    json: &serde_json::Value,
    entries: &mut Vec<AgentLogEntry>,
    last_user_timestamp: &mut Option<String>,
) {
    let record_type = get_str(json, "type").unwrap_or("");

    if record_type == "user" {
        if let Some(ts) = extract_timestamp(json) {
            *last_user_timestamp = Some(ts);
        }
        return;
    }

    if let Some(error_msg) = extract_error(json) {
        let timestamp = extract_timestamp(json).unwrap_or_default();
        entries.push(AgentLogEntry { timestamp, kind: EntryKind::Error { message: error_msg } });
        return;
    }

    if record_type == "assistant" {
        let message = match json.get("message") {
            Some(m) => m,
            None => return,
        };

        if let Some(content) = message.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if get_str(block, "type") != Some("tool_use") {
                    continue;
                }
                let Some(tool_name) = get_str(block, "name") else {
                    continue;
                };
                let input = block.get("input").cloned().unwrap_or(serde_json::Value::Null);
                let timestamp = extract_timestamp(json).unwrap_or_default();

                let kind = match tool_name {
                    "Read" | "Edit" | "NotebookEdit" => {
                        let key =
                            if tool_name == "NotebookEdit" { "notebook_path" } else { "file_path" };
                        let Some(path) = get_str(&input, key) else {
                            continue;
                        };
                        let path = path.to_string();
                        match tool_name {
                            "Read" => EntryKind::FileRead { path },
                            "Edit" => EntryKind::FileEdit { path },
                            _ => EntryKind::NotebookEdit { path },
                        }
                    }
                    "Write" => {
                        let Some(path) = get_str(&input, "file_path") else {
                            continue;
                        };
                        let lines = get_str(&input, "content").unwrap_or("").lines().count();
                        EntryKind::FileWrite { path: path.to_string(), new: true, lines }
                    }
                    "Bash" => {
                        let command = get_str(&input, "command").unwrap_or("").to_string();
                        let trimmed = command.trim();
                        if trimmed == "oj"
                            || trimmed.starts_with("oj ")
                            || trimmed.starts_with("./oj ")
                        {
                            let rest = trimmed
                                .strip_prefix("./oj ")
                                .or_else(|| trimmed.strip_prefix("oj "));
                            let args = rest
                                .map(|r| r.split_whitespace().map(String::from).collect())
                                .unwrap_or_default();
                            EntryKind::OjCall { args }
                        } else {
                            let cmd = if command.len() > 80 {
                                format!("{}...", &command[..77])
                            } else {
                                command
                            };
                            EntryKind::BashCommand { command: cmd, exit_code: None }
                        }
                    }
                    _ => continue,
                };
                entries.push(AgentLogEntry { timestamp, kind });
            }
        }

        if get_str(message, "stop_reason").unwrap_or("") == "end_turn" {
            let timestamp = extract_timestamp(json).unwrap_or_default();
            let tokens =
                message.get("usage").and_then(|u| u.get("output_tokens")).and_then(|t| t.as_u64());
            let duration_secs =
                compute_duration_secs(last_user_timestamp.as_deref(), timestamp.as_str());
            entries.push(AgentLogEntry {
                timestamp,
                kind: EntryKind::TurnComplete { duration_secs, tokens },
            });
        }
    }

    if record_type == "result" {
        if let Some(exit_code) = get_str(json, "content").and_then(extract_exit_code) {
            for entry in entries.iter_mut().rev() {
                if let EntryKind::BashCommand { exit_code: ref mut ec, .. } = entry.kind {
                    if ec.is_none() {
                        *ec = Some(exit_code);
                        break;
                    }
                }
            }
        }
    }
}

/// Extract a timestamp from a JSONL record.
fn extract_timestamp(json: &serde_json::Value) -> Option<String> {
    get_str(json, "timestamp")
        .or_else(|| json.get("message").and_then(|m| get_str(m, "created_at")))
        .or_else(|| get_str(json, "isoTimestamp"))
        .map(String::from)
}

/// Extract error message from a JSONL record.
fn extract_error(json: &serde_json::Value) -> Option<String> {
    get_str(json, "error")
        .or_else(|| json.get("message").and_then(|m| get_str(m, "error")))
        .map(String::from)
}

/// Compute duration between two ISO 8601 timestamps in seconds.
fn compute_duration_secs(start: Option<&str>, end: &str) -> Option<u64> {
    let start_secs = parse_iso_epoch(start?)?;
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

/// Extract exit code from tool result content (e.g. "exit code: 0").
fn extract_exit_code(content: &str) -> Option<i32> {
    let pos = content.to_lowercase().find("exit code:")?;
    let after = content[pos + 10..].trim();
    after.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect::<String>().parse().ok()
}

#[cfg(test)]
#[path = "log_entry_tests.rs"]
mod tests;
