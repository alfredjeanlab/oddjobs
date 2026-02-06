// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn parse_working_state() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"type":"user","message":{"content":"test"}}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_waiting_state_text_only() {
    // Assistant message with only text content = waiting for input
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
}

#[test]
fn parse_tool_use_state() {
    // Assistant message with tool_use = working
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_thinking_block_as_working() {
    // Assistant message with thinking content = still working (not idle)
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Let me analyze..."}]}}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_thinking_with_text_as_working() {
    // Assistant message with thinking + text (no tool_use) = still working
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"I'll do that"}]}}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_empty_content_as_waiting() {
    // Assistant message with no content = waiting for input
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[]}}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
}

#[test]
fn parse_rate_limit_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"error":"Rate limit exceeded"}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::RateLimited));
}

#[test]
fn parse_unauthorized_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"error":"Invalid API key - unauthorized"}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::Unauthorized));
}

#[test]
fn parse_empty_file() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, "").unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_missing_file() {
    let state = parse_session_log(Path::new("/nonexistent/path.jsonl"));
    assert_eq!(state, AgentState::Working);
}

// --- Additional Error Detection Tests ---

#[test]
fn parse_out_of_credits_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"error":"Your account has run out of credits"}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::OutOfCredits));
}

#[test]
fn parse_quota_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"error":"Quota exceeded for this month"}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::OutOfCredits));
}

#[test]
fn parse_billing_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"error":"Billing issue detected"}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::OutOfCredits));
}

#[test]
fn parse_network_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"error":"Network error occurred"}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::NoInternet));
}

#[test]
fn parse_connection_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"error":"Connection refused"}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::NoInternet));
}

#[test]
fn parse_offline_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"error":"You appear to be offline"}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::NoInternet));
}

#[test]
fn parse_too_many_requests_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"error":"Too many requests, please slow down"}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::RateLimited));
}

#[test]
fn parse_generic_error() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, r#"{"error":"Something unexpected happened"}"#).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(
        state,
        AgentState::Failed(AgentError::Other(
            "Something unexpected happened".to_string()
        ))
    );
}

#[test]
fn parse_error_in_message_field() {
    // Error can also be nested in message.error
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"error":"Invalid API key"}}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Failed(AgentError::Unauthorized));
}

// --- Stop Reason Tests ---

#[test]
fn parse_non_null_stop_reason_as_working() {
    // When stop_reason is non-null (unexpected), we log a warning and treat as working
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_null_stop_reason_as_normal() {
    // Null stop_reason is the normal case - should parse content to determine state
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"stop_reason":null,"content":[{"type":"text","text":"Done"}]}}"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
}

// --- Project Dir Name Tests ---

#[test]
fn project_dir_name_replaces_slashes_and_dots() {
    // Note: project_dir_name canonicalizes paths, so we need to use a real path
    let dir = TempDir::new().unwrap();
    let result = project_dir_name(dir.path());
    // Should not contain any slashes or dots (except possibly at start for root)
    assert!(!result.contains('/'), "should replace slashes with dashes");
    // The path should contain dashes where slashes were
    assert!(
        result.contains('-'),
        "should have dashes from path separators"
    );
}

// --- Incomplete JSON / Edge Case Tests ---

#[test]
fn parse_incomplete_json_line_does_not_crash() {
    // Incomplete JSON at EOF should not cause a crash - treated as working
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    // Write a complete line followed by an incomplete line (no closing brace)
    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}
{"type":"assistant","message":{"content":[{"type":"text"#,
    )
    .unwrap();

    // Should not panic, should return Working (last complete line was user message)
    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_malformed_json_line_does_not_crash() {
    // Invalid JSON should not crash - treated as working
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(&log_path, "this is not valid json\n").unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_empty_json_object_does_not_crash() {
    // Empty JSON object should not crash
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(&log_path, "{}\n").unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_binary_garbage_does_not_crash() {
    // Binary data should not crash the parser
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    // Write some binary garbage
    std::fs::write(&log_path, &[0x00, 0x01, 0x02, 0xFF, 0xFE, 0x0A]).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_very_long_line_does_not_crash() {
    // Very long line should be handled without crash
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    // Create a very long but valid JSON line
    let long_text = "x".repeat(100_000);
    let content = format!(
        r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{}"}}]}}}}
"#,
        long_text
    );
    std::fs::write(&log_path, content).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::WaitingForInput);
}
