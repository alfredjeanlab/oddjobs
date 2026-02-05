// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

fn jsonl_lines(lines: &[&str]) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    for line in lines {
        writeln!(file, "{}", line).unwrap();
    }
    file.flush().unwrap();
    file
}

#[test]
fn parse_read_tool() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/src/main.rs"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:05Z"}"#,
    ]);

    let (entries, offset) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::FileRead {
            path: "/src/main.rs".to_string()
        }
    );
    assert_eq!(entries[0].timestamp, "2026-01-30T08:17:05Z");
    assert!(offset > 0);
}

#[test]
fn parse_write_tool() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/src/lib.rs","content":"line1\nline2\nline3\n"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:10Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::FileWrite {
            path: "/src/lib.rs".to_string(),
            new: true,
            lines: 3,
        }
    );
}

#[test]
fn parse_edit_tool() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/src/mod.rs","old_string":"foo","new_string":"bar"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:12Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::FileEdit {
            path: "/src/mod.rs".to_string()
        }
    );
}

#[test]
fn parse_notebook_edit_tool() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"NotebookEdit","input":{"notebook_path":"/notebooks/analysis.ipynb","new_source":"print('hi')"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:14Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::NotebookEdit {
            path: "/notebooks/analysis.ipynb".to_string()
        }
    );
}

#[test]
fn parse_bash_command() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo build -p oj"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:30Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::BashCommand {
            command: "cargo build -p oj".to_string(),
            exit_code: None,
        }
    );
}

#[test]
fn parse_oj_call() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"oj job list"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:45Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::OjCall {
            args: vec!["job".to_string(), "list".to_string()],
        }
    );
}

#[test]
fn parse_oj_call_with_path_prefix() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"./oj session list --json"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:45Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::OjCall {
            args: vec![
                "session".to_string(),
                "list".to_string(),
                "--json".to_string()
            ],
        }
    );
}

#[test]
fn parse_turn_complete() {
    let file = jsonl_lines(&[
        r#"{"type":"user","message":{"content":[{"type":"text","text":"hello"}]},"isoTimestamp":"2026-01-30T08:17:00Z"}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"response"}],"stop_reason":"end_turn","usage":{"output_tokens":1500}},"isoTimestamp":"2026-01-30T08:17:58Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::TurnComplete {
            duration_secs: Some(58),
            tokens: Some(1500),
        }
    );
}

#[test]
fn parse_error_entry() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","error":"rate limited","isoTimestamp":"2026-01-30T08:17:51Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::Error {
            message: "rate limited".to_string(),
        }
    );
}

#[test]
fn parse_incremental() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/a.rs"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:05Z"}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/b.rs"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:06Z"}"#,
    ]);

    // Parse first batch
    let (entries1, offset1) = parse_entries_from(file.path(), 0);
    assert_eq!(entries1.len(), 2);

    // No new data — should return empty
    let (entries2, offset2) = parse_entries_from(file.path(), offset1);
    assert_eq!(entries2.len(), 0);
    assert_eq!(offset2, offset1);
}

#[test]
fn parse_multiple_tools_in_one_message() {
    let file = jsonl_lines(&[
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/a.rs"}},{"type":"tool_use","name":"Read","input":{"file_path":"/b.rs"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:05Z"}"#,
    ]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].kind,
        EntryKind::FileRead {
            path: "/a.rs".to_string()
        }
    );
    assert_eq!(
        entries[1].kind,
        EntryKind::FileRead {
            path: "/b.rs".to_string()
        }
    );
}

#[test]
fn parse_user_message_is_skipped() {
    let file = jsonl_lines(&[
        r#"{"type":"user","message":{"content":[{"type":"text","text":"hello"}]},"isoTimestamp":"2026-01-30T08:17:00Z"}"#,
    ]);

    let (entries, offset) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 0);
    assert!(offset > 0); // Offset still advances past the line
}

#[test]
fn parse_missing_file_returns_empty() {
    let (entries, offset) = parse_entries_from(Path::new("/nonexistent/path.jsonl"), 0);
    assert_eq!(entries.len(), 0);
    assert_eq!(offset, 0);
}

#[test]
fn display_entry_kinds() {
    assert_eq!(
        EntryKind::FileRead {
            path: "/a.rs".to_string()
        }
        .to_string(),
        "read: /a.rs"
    );
    assert_eq!(
        EntryKind::FileWrite {
            path: "/b.rs".to_string(),
            new: true,
            lines: 15,
        }
        .to_string(),
        "wrote: /b.rs (new, 15 lines)"
    );
    assert_eq!(
        EntryKind::FileWrite {
            path: "/c.rs".to_string(),
            new: false,
            lines: 10,
        }
        .to_string(),
        "wrote: /c.rs (10 lines)"
    );
    assert_eq!(
        EntryKind::FileEdit {
            path: "/d.rs".to_string(),
        }
        .to_string(),
        "edited: /d.rs"
    );
    assert_eq!(
        EntryKind::BashCommand {
            command: "cargo build".to_string(),
            exit_code: Some(0),
        }
        .to_string(),
        "bash: cargo build (exit 0)"
    );
    assert_eq!(
        EntryKind::OjCall {
            args: vec!["job".to_string(), "list".to_string()],
        }
        .to_string(),
        "oj: job list"
    );
    assert_eq!(
        EntryKind::TurnComplete {
            duration_secs: Some(58),
            tokens: Some(1500),
        }
        .to_string(),
        "turn complete (58s, 1.5k tokens)"
    );
    assert_eq!(
        EntryKind::Error {
            message: "rate limited".to_string(),
        }
        .to_string(),
        "error: rate limited"
    );
}

#[test]
fn format_tokens_display() {
    assert_eq!(format_tokens(500), "500 tokens");
    assert_eq!(format_tokens(1000), "1k tokens");
    assert_eq!(format_tokens(1500), "1.5k tokens");
    assert_eq!(format_tokens(2000), "2k tokens");
    assert_eq!(format_tokens(10500), "10.5k tokens");
}

#[test]
fn parse_long_bash_command_truncated() {
    let long_cmd = "a".repeat(100);
    let json_line = format!(
        r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Bash","input":{{"command":"{}"}}}}],"stop_reason":"tool_use"}},"isoTimestamp":"2026-01-30T08:17:30Z"}}"#,
        long_cmd
    );
    let file = jsonl_lines(&[&json_line]);

    let (entries, _) = parse_entries_from(file.path(), 0);
    assert_eq!(entries.len(), 1);
    if let EntryKind::BashCommand { command, .. } = &entries[0].kind {
        assert_eq!(command.len(), 80); // 77 + "..."
        assert!(command.ends_with("..."));
    } else {
        panic!("expected BashCommand");
    }
}
