// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn incremental_parser_reads_only_new_content() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    // Write initial content
    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}
"#,
    )
    .unwrap();

    let mut parser = SessionLogParser::new();
    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::Working);
    assert!(parser.last_offset > 0, "offset should advance");

    let offset_after_first = parser.last_offset;

    // Append new content
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .unwrap();
    writeln!(
        file,
        r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"Done!"}}]}}}}"#,
    )
    .unwrap();

    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
    assert!(
        parser.last_offset > offset_after_first,
        "offset should advance past appended content"
    );
}

#[test]
fn incremental_parser_returns_cached_state_when_no_new_content() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}
"#,
    )
    .unwrap();

    let mut parser = SessionLogParser::new();
    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);

    // Parse again with no new content — should return same state from cache
    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
}

#[test]
fn incremental_parser_handles_file_truncation() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    // Write a long initial log
    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}
"#,
    )
    .unwrap();

    let mut parser = SessionLogParser::new();
    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
    let large_offset = parser.last_offset;

    // Truncate and write shorter content (simulates log file being replaced)
    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"retry"}}
"#,
    )
    .unwrap();

    // File is now shorter than last_offset — parser should reset
    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::Working);
    assert!(
        parser.last_offset < large_offset,
        "offset should reset after truncation"
    );
}

#[test]
fn incremental_parser_handles_multiple_appends() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}
"#,
    )
    .unwrap();

    let mut parser = SessionLogParser::new();
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Append assistant thinking (working)
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .unwrap();
    writeln!(
        file,
        r#"{{"type":"assistant","message":{{"content":[{{"type":"thinking","thinking":"..."}}]}}}}"#,
    )
    .unwrap();

    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Append tool use result (working — user message)
    writeln!(
        file,
        r#"{{"type":"user","message":{{"content":"tool result"}}}}"#,
    )
    .unwrap();

    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Append final text-only response (idle)
    writeln!(
        file,
        r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"All done"}}]}}}}"#,
    )
    .unwrap();

    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
}

#[test]
fn incremental_parser_handles_incomplete_final_line() {
    // Parser should not advance offset past incomplete lines
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    // Write complete line with newline
    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}
"#,
    )
    .unwrap();

    let mut parser = SessionLogParser::new();
    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::Working);
    let offset_after_complete = parser.last_offset;

    // Append incomplete line (no trailing newline)
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .unwrap();
    write!(
        file,
        r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"partial"#
    )
    .unwrap();

    // Parser should still work and not advance offset past incomplete line
    let state = parser.parse(&log_path);
    // The incomplete line is parsed but offset not advanced
    assert_eq!(parser.last_offset, offset_after_complete);
    // State should reflect the user message (last complete line) or working
    assert_eq!(state, AgentState::Working);

    // Now complete the line - use write_all to avoid format string escaping issues
    // Complete JSON: {"type":"assistant","message":{"content":[{"type":"text","text":"partial"}]}}
    file.write_all(b"\"}]}}\n").unwrap();

    // Now parser should see the complete line
    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
    assert!(
        parser.last_offset > offset_after_complete,
        "offset should advance after line is complete"
    );
}

#[test]
fn rapid_state_changes_all_detected() {
    // Simulate rapid appends and verify each state is parseable
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(&log_path, "").unwrap();

    let mut parser = SessionLogParser::new();

    // Initial empty = working
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // User message = working
    append_line(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Assistant with tool_use = working
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // User (tool result) = working
    append_line(
        &log_path,
        r#"{"type":"user","message":{"content":"tool result"}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Assistant with text only = idle
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);

    // User message again = back to working
    append_line(
        &log_path,
        r#"{"type":"user","message":{"content":"continue"}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Assistant with thinking = working (not idle)
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Let me think..."}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Finally text only = idle again
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"All done"}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
}
