// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

// --- Basic State Parsing (parameterized) ---

#[yare::parameterized(
    working_user_message = {
        r#"{"type":"user","message":{"content":"test"}}"#,
        AgentState::Working,
    },
    waiting_text_only = {
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
        AgentState::WaitingForInput,
    },
    working_tool_use = {
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#,
        AgentState::Working,
    },
    working_thinking_block = {
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Let me analyze..."}]}}"#,
        AgentState::Working,
    },
    working_thinking_with_text = {
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"I'll do that"}]}}"#,
        AgentState::Working,
    },
    waiting_empty_content = {
        r#"{"type":"assistant","message":{"content":[]}}"#,
        AgentState::WaitingForInput,
    },
    working_empty_file = { "", AgentState::Working },
    working_empty_json_object = { "{}\n", AgentState::Working },
)]
fn parse_session_state(content: &str, expected: AgentState) {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, content).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, expected);
}

#[test]
fn parse_missing_file() {
    let state = parse_session_log(Path::new("/nonexistent/path.jsonl"));
    assert_eq!(state, AgentState::Working);
}

// --- Error Classification (parameterized) ---

#[yare::parameterized(
    rate_limit = {
        r#"{"error":"Rate limit exceeded"}"#,
        AgentState::Failed(AgentError::RateLimited),
    },
    too_many_requests = {
        r#"{"error":"Too many requests, please slow down"}"#,
        AgentState::Failed(AgentError::RateLimited),
    },
    unauthorized = {
        r#"{"error":"Invalid API key - unauthorized"}"#,
        AgentState::Failed(AgentError::Unauthorized),
    },
    out_of_credits = {
        r#"{"error":"Your account has run out of credits"}"#,
        AgentState::Failed(AgentError::OutOfCredits),
    },
    quota_exceeded = {
        r#"{"error":"Quota exceeded for this month"}"#,
        AgentState::Failed(AgentError::OutOfCredits),
    },
    billing_issue = {
        r#"{"error":"Billing issue detected"}"#,
        AgentState::Failed(AgentError::OutOfCredits),
    },
    network_error = {
        r#"{"error":"Network error occurred"}"#,
        AgentState::Failed(AgentError::NoInternet),
    },
    connection_refused = {
        r#"{"error":"Connection refused"}"#,
        AgentState::Failed(AgentError::NoInternet),
    },
    offline = {
        r#"{"error":"You appear to be offline"}"#,
        AgentState::Failed(AgentError::NoInternet),
    },
    generic_error = {
        r#"{"error":"Something unexpected happened"}"#,
        AgentState::Failed(AgentError::Other("Something unexpected happened".to_string())),
    },
    error_in_message_field = {
        r#"{"type":"assistant","message":{"error":"Invalid API key"}}"#,
        AgentState::Failed(AgentError::Unauthorized),
    },
)]
fn parse_error_state(content: &str, expected: AgentState) {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, content).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, expected);
}

// --- Stop Reason Tests ---

#[yare::parameterized(
    non_null_stop_reason_as_working = {
        r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}"#,
        AgentState::Working,
    },
    null_stop_reason_as_normal = {
        r#"{"type":"assistant","message":{"stop_reason":null,"content":[{"type":"text","text":"Done"}]}}"#,
        AgentState::WaitingForInput,
    },
)]
fn parse_stop_reason(content: &str, expected: AgentState) {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, content).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, expected);
}

// --- Edge Case / Robustness Tests ---

#[test]
fn parse_incomplete_json_line_does_not_crash() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}
{"type":"assistant","message":{"content":[{"type":"text"#,
    )
    .unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_malformed_json_line_does_not_crash() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(&log_path, "this is not valid json\n").unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_binary_garbage_does_not_crash() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

    std::fs::write(&log_path, &[0x00, 0x01, 0x02, 0xFF, 0xFE, 0x0A]).unwrap();

    let state = parse_session_log(&log_path);
    assert_eq!(state, AgentState::Working);
}

#[test]
fn parse_very_long_line_does_not_crash() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");

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

// --- Project Dir Name ---

#[test]
fn project_dir_name_replaces_slashes_and_dots() {
    let dir = TempDir::new().unwrap();
    let result = project_dir_name(dir.path());
    assert!(!result.contains('/'), "should replace slashes with dashes");
    assert!(
        result.contains('-'),
        "should have dashes from path separators"
    );
}

// --- find_session_log Correctness ---

#[test]
fn find_session_log_requires_correct_workspace_path() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();

    let session_id = "test-session";

    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join(format!("{session_id}.jsonl")),
        r#"{"type":"user","message":{"content":"hello"}}"#,
    )
    .unwrap();

    assert!(
        find_session_log_in(workspace_dir.path(), session_id, claude_base.path()).is_some(),
        "should find session log when given the workspace path"
    );

    assert!(
        find_session_log_in(project_dir.path(), session_id, claude_base.path()).is_none(),
        "should not find session log when given project_root (different hash)"
    );
}
