// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

fn extract(json_str: &str) -> Vec<AgentLogEntry> {
    let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
    let mut entries = Vec::new();
    let mut last_user_timestamp: Option<String> = None;
    extract_entries(&json, &mut entries, &mut last_user_timestamp);
    entries
}

fn extract_with_user_ts(user_json: &str, assistant_json: &str) -> Vec<AgentLogEntry> {
    let mut entries = Vec::new();
    let mut last_user_timestamp: Option<String> = None;
    let user: serde_json::Value = serde_json::from_str(user_json).unwrap();
    extract_entries(&user, &mut entries, &mut last_user_timestamp);
    let assistant: serde_json::Value = serde_json::from_str(assistant_json).unwrap();
    extract_entries(&assistant, &mut entries, &mut last_user_timestamp);
    entries
}

#[test]
fn extract_read_tool() {
    let entries = extract(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/src/main.rs"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:05Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, EntryKind::FileRead { path: "/src/main.rs".to_string() });
    assert_eq!(entries[0].timestamp, "2026-01-30T08:17:05Z");
}

#[test]
fn extract_write_tool() {
    let entries = extract(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/src/lib.rs","content":"line1\nline2\nline3\n"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:10Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::FileWrite { path: "/src/lib.rs".to_string(), new: true, lines: 3 }
    );
}

#[test]
fn extract_edit_tool() {
    let entries = extract(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/src/mod.rs","old_string":"foo","new_string":"bar"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:12Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, EntryKind::FileEdit { path: "/src/mod.rs".to_string() });
}

#[test]
fn extract_notebook_edit_tool() {
    let entries = extract(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"NotebookEdit","input":{"notebook_path":"/notebooks/analysis.ipynb","new_source":"print('hi')"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:14Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::NotebookEdit { path: "/notebooks/analysis.ipynb".to_string() }
    );
}

#[test]
fn extract_bash_command() {
    let entries = extract(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo build -p oj"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:30Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::BashCommand { command: "cargo build -p oj".to_string(), exit_code: None }
    );
}

#[test]
fn extract_oj_call() {
    let entries = extract(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"oj job list"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:45Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::OjCall { args: vec!["job".to_string(), "list".to_string()] }
    );
}

#[test]
fn extract_oj_call_with_path_prefix() {
    let entries = extract(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"./oj session list --json"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:45Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::OjCall {
            args: vec!["session".to_string(), "list".to_string(), "--json".to_string()],
        }
    );
}

#[test]
fn extract_turn_complete() {
    let entries = extract_with_user_ts(
        r#"{"type":"user","message":{"content":[{"type":"text","text":"hello"}]},"isoTimestamp":"2026-01-30T08:17:00Z"}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"response"}],"stop_reason":"end_turn","usage":{"output_tokens":1500}},"isoTimestamp":"2026-01-30T08:17:58Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        EntryKind::TurnComplete { duration_secs: Some(58), tokens: Some(1500) }
    );
}

#[test]
fn extract_error_entry() {
    let entries = extract(
        r#"{"type":"assistant","error":"rate limited","isoTimestamp":"2026-01-30T08:17:51Z"}"#,
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, EntryKind::Error { message: "rate limited".to_string() });
}

#[test]
fn extract_multiple_tools_in_one_message() {
    let entries = extract(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/a.rs"}},{"type":"tool_use","name":"Read","input":{"file_path":"/b.rs"}}],"stop_reason":"tool_use"},"isoTimestamp":"2026-01-30T08:17:05Z"}"#,
    );
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].kind, EntryKind::FileRead { path: "/a.rs".to_string() });
    assert_eq!(entries[1].kind, EntryKind::FileRead { path: "/b.rs".to_string() });
}

#[test]
fn extract_user_message_produces_no_entries() {
    let entries = extract(
        r#"{"type":"user","message":{"content":[{"type":"text","text":"hello"}]},"isoTimestamp":"2026-01-30T08:17:00Z"}"#,
    );
    assert_eq!(entries.len(), 0);
}

#[test]
fn display_entry_kinds() {
    assert_eq!(EntryKind::FileRead { path: "/a.rs".to_string() }.to_string(), "read: /a.rs");
    assert_eq!(
        EntryKind::FileWrite { path: "/b.rs".to_string(), new: true, lines: 15 }.to_string(),
        "wrote: /b.rs (new, 15 lines)"
    );
    assert_eq!(
        EntryKind::FileWrite { path: "/c.rs".to_string(), new: false, lines: 10 }.to_string(),
        "wrote: /c.rs (10 lines)"
    );
    assert_eq!(EntryKind::FileEdit { path: "/d.rs".to_string() }.to_string(), "edited: /d.rs");
    assert_eq!(
        EntryKind::BashCommand { command: "cargo build".to_string(), exit_code: Some(0) }
            .to_string(),
        "bash: cargo build (exit 0)"
    );
    assert_eq!(
        EntryKind::OjCall { args: vec!["job".to_string(), "list".to_string()] }.to_string(),
        "oj: job list"
    );
    assert_eq!(
        EntryKind::TurnComplete { duration_secs: Some(58), tokens: Some(1500) }.to_string(),
        "turn complete (58s, 1.5k tokens)"
    );
    assert_eq!(
        EntryKind::Error { message: "rate limited".to_string() }.to_string(),
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
fn extract_long_bash_command_truncated() {
    let long_cmd = "a".repeat(100);
    let json_line = format!(
        r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Bash","input":{{"command":"{}"}}}}],"stop_reason":"tool_use"}},"isoTimestamp":"2026-01-30T08:17:30Z"}}"#,
        long_cmd
    );
    let entries = extract(&json_line);
    assert_eq!(entries.len(), 1);
    if let EntryKind::BashCommand { command, .. } = &entries[0].kind {
        assert_eq!(command.len(), 80); // 77 + "..."
        assert!(command.ends_with("..."));
    } else {
        panic!("expected BashCommand");
    }
}
