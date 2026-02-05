// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn reads_only_new_content() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

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
fn returns_cached_state_when_no_new_content() {
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

    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
}

#[test]
fn handles_file_truncation() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

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

    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"retry"}}
"#,
    )
    .unwrap();

    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::Working);
    assert!(
        parser.last_offset < large_offset,
        "offset should reset after truncation"
    );
}

#[test]
fn handles_multiple_appends() {
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

    writeln!(
        file,
        r#"{{"type":"user","message":{{"content":"tool result"}}}}"#,
    )
    .unwrap();

    assert_eq!(parser.parse(&log_path), AgentState::Working);

    writeln!(
        file,
        r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"All done"}}]}}}}"#,
    )
    .unwrap();

    assert_eq!(parser.parse(&log_path), AgentState::WaitingForInput);
}

#[test]
fn handles_incomplete_final_line() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

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

    // Complete the line
    file.write_all(b"\"}]}}\n").unwrap();

    let state = parser.parse(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
    assert!(
        parser.last_offset > offset_after_complete,
        "offset should advance after line is complete"
    );
}

#[test]
fn rapid_state_changes_all_detected() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(&log_path, "").unwrap();

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
