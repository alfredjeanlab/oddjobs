// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn incremental_parser_reads_only_new_content() {
    let (_dir, log_path) = temp_log(
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    );

    let mut parser = SessionLogParser::new();
    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::Working);
    assert!(parser.last_offset > 0, "offset should advance");

    let offset_after_first = parser.last_offset;

    // Append new content
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    );

    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
    assert!(
        parser.last_offset > offset_after_first,
        "offset should advance past appended content"
    );
}

#[test]
fn incremental_parser_returns_cached_state_when_no_new_content() {
    let (_dir, log_path) = temp_log(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Done!\"}]}}\n",
    );

    let mut parser = SessionLogParser::new();
    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
}

#[test]
fn incremental_parser_handles_file_truncation() {
    let (_dir, log_path) = temp_log(
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n\
         {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Done!\"}]}}\n",
    );

    let mut parser = SessionLogParser::new();
    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
    let large_offset = parser.last_offset;

    // Truncate and write shorter content
    std::fs::write(
        &log_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"retry\"}}\n",
    )
    .unwrap();

    assert_eq!(parser.parse(&log_path), AgentState::Working);
    assert!(
        parser.last_offset < large_offset,
        "offset should reset after truncation"
    );
}

#[test]
fn incremental_parser_handles_multiple_appends() {
    let (_dir, log_path) = temp_log(
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    );

    let mut parser = SessionLogParser::new();
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Append assistant thinking (working)
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"..."}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Append tool use result (working — user message)
    append_line(
        &log_path,
        r#"{"type":"user","message":{"content":"tool result"}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    // Append final text-only response (idle)
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"All done"}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
}

#[test]
fn incremental_parser_handles_incomplete_final_line() {
    let (_dir, log_path) = temp_log(
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    );

    let mut parser = SessionLogParser::new();
    assert_eq!(parser.parse(&log_path), AgentState::Working);
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

    let state = parser.parse(&log_path);
    assert_eq!(parser.last_offset, offset_after_complete);
    assert_eq!(state, AgentState::Working);

    // Now complete the line
    file.write_all(b"\"}]}}\n").unwrap();

    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
    assert!(
        parser.last_offset > offset_after_complete,
        "offset should advance after line is complete"
    );
}

#[test]
fn rapid_state_changes_all_detected() {
    let (_dir, log_path) = temp_log("");
    let mut parser = SessionLogParser::new();

    assert_eq!(parser.parse(&log_path), AgentState::Working);

    append_line(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    append_line(
        &log_path,
        r#"{"type":"user","message":{"content":"tool result"}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);

    append_line(
        &log_path,
        r#"{"type":"user","message":{"content":"continue"}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Let me think..."}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::Working);

    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"All done"}]}}"#,
    );
    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
}
