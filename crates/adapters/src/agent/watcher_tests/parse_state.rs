// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[yare::parameterized(
    working_user_message          = { r#"{"type":"user","message":{"content":"test"}}"#, AgentState::Working },
    waiting_text_only             = { r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#, AgentState::WaitingForInput },
    working_tool_use              = { r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#, AgentState::Working },
    working_thinking              = { r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"Let me analyze..."}]}}"#, AgentState::Working },
    working_thinking_with_text    = { r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"I'll do that"}]}}"#, AgentState::Working },
    waiting_empty_content         = { r#"{"type":"assistant","message":{"content":[]}}"#, AgentState::WaitingForInput },
    working_empty_file            = { "", AgentState::Working },
    working_non_null_stop_reason  = { r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}"#, AgentState::Working },
    waiting_null_stop_reason      = { r#"{"type":"assistant","message":{"stop_reason":null,"content":[{"type":"text","text":"Done"}]}}"#, AgentState::WaitingForInput },
)]
fn parse_state_from_content(content: &str, expected: AgentState) {
    let (_dir, log_path) = temp_log(content);
    assert_eq!(parse_session_log(&log_path), expected);
}

#[test]
fn parse_missing_file() {
    let state = parse_session_log(Path::new("/nonexistent/path.jsonl"));
    assert_eq!(state, AgentState::Working);
}

#[yare::parameterized(
    rate_limit       = { r#"{"error":"Rate limit exceeded"}"#,                    AgentState::Failed(AgentError::RateLimited) },
    unauthorized     = { r#"{"error":"Invalid API key - unauthorized"}"#,          AgentState::Failed(AgentError::Unauthorized) },
    out_of_credits   = { r#"{"error":"Your account has run out of credits"}"#,     AgentState::Failed(AgentError::OutOfCredits) },
    quota            = { r#"{"error":"Quota exceeded for this month"}"#,           AgentState::Failed(AgentError::OutOfCredits) },
    billing          = { r#"{"error":"Billing issue detected"}"#,                  AgentState::Failed(AgentError::OutOfCredits) },
    network          = { r#"{"error":"Network error occurred"}"#,                  AgentState::Failed(AgentError::NoInternet) },
    connection       = { r#"{"error":"Connection refused"}"#,                      AgentState::Failed(AgentError::NoInternet) },
    offline          = { r#"{"error":"You appear to be offline"}"#,                AgentState::Failed(AgentError::NoInternet) },
    too_many_requests = { r#"{"error":"Too many requests, please slow down"}"#,    AgentState::Failed(AgentError::RateLimited) },
    error_in_message = { r#"{"type":"assistant","message":{"error":"Invalid API key"}}"#, AgentState::Failed(AgentError::Unauthorized) },
)]
fn parse_error_classification(content: &str, expected: AgentState) {
    let (_dir, log_path) = temp_log(content);
    assert_eq!(parse_session_log(&log_path), expected);
}

#[test]
fn parse_generic_error() {
    let (_dir, log_path) = temp_log(r#"{"error":"Something unexpected happened"}"#);
    assert_eq!(
        parse_session_log(&log_path),
        AgentState::Failed(AgentError::Other(
            "Something unexpected happened".to_string()
        ))
    );
}

// --- Project Dir Name Tests ---

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

// --- Incomplete JSON / Edge Case Tests ---

#[test]
fn parse_incomplete_json_line_does_not_crash() {
    let (_dir, log_path) = temp_log(
        r#"{"type":"user","message":{"content":"hello"}}
{"type":"assistant","message":{"content":[{"type":"text"#,
    );
    assert_eq!(parse_session_log(&log_path), AgentState::Working);
}

#[test]
fn parse_malformed_json_line_does_not_crash() {
    let (_dir, log_path) = temp_log("this is not valid json\n");
    assert_eq!(parse_session_log(&log_path), AgentState::Working);
}

#[test]
fn parse_empty_json_object_does_not_crash() {
    let (_dir, log_path) = temp_log("{}\n");
    assert_eq!(parse_session_log(&log_path), AgentState::Working);
}

#[test]
fn parse_binary_garbage_does_not_crash() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, &[0x00, 0x01, 0x02, 0xFF, 0xFE, 0x0A]).unwrap();
    assert_eq!(parse_session_log(&log_path), AgentState::Working);
}

#[test]
fn parse_very_long_line_does_not_crash() {
    let long_text = "x".repeat(100_000);
    let content = format!(
        r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{}"}}]}}}}
"#,
        long_text
    );
    let (_dir, log_path) = temp_log(&content);
    assert_eq!(parse_session_log(&log_path), AgentState::WaitingForInput);
}
