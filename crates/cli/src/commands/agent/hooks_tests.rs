// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn notification_hook_input_parses_idle_prompt() {
    let json =
        r#"{"session_id":"abc","notification_type":"idle_prompt","message":"Claude needs input"}"#;
    let input: NotificationHookInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.notification_type, "idle_prompt");
}

#[test]
fn notification_hook_input_parses_permission_prompt() {
    let json = r#"{"session_id":"abc","notification_type":"permission_prompt","message":"Permission needed"}"#;
    let input: NotificationHookInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.notification_type, "permission_prompt");
}

#[test]
fn notification_hook_input_parses_unknown_type() {
    let json = r#"{"session_id":"abc","notification_type":"auth_success"}"#;
    let input: NotificationHookInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.notification_type, "auth_success");
}

#[test]
fn notification_hook_input_handles_missing_type() {
    let json = r#"{"session_id":"abc"}"#;
    let input: NotificationHookInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.notification_type, "");
}

// =============================================================================
// StopHookInput parsing
// =============================================================================

#[test]
fn stop_hook_input_parses_transcript_path() {
    let json = r#"{"stop_hook_active":false,"transcript_path":"/tmp/session.jsonl"}"#;
    let input: StopHookInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.transcript_path.as_deref(), Some("/tmp/session.jsonl"));
    assert!(!input.stop_hook_active);
}

#[test]
fn stop_hook_input_handles_missing_transcript_path() {
    let json = r#"{"stop_hook_active":true}"#;
    let input: StopHookInput = serde_json::from_str(json).unwrap();
    assert!(input.transcript_path.is_none());
    assert!(input.stop_hook_active);
}

// =============================================================================
// has_unrecoverable_error
// =============================================================================

fn temp_transcript(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("session.jsonl");
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

#[yare::parameterized(
    rate_limit          = { r#"{"error":"Rate limit exceeded"}"#, true },
    too_many_requests   = { r#"{"error":"Too many requests, please slow down"}"#, true },
    out_of_credits      = { r#"{"error":"Your account has run out of credits"}"#, true },
    billing_quota       = { r#"{"error":"Billing quota exceeded"}"#, true },
    unauthorized        = { r#"{"error":"Unauthorized access"}"#, true },
    invalid_api_key     = { r#"{"error":"Invalid API key provided"}"#, true },
    error_in_message    = { r#"{"type":"assistant","message":{"error":"Rate limit exceeded"}}"#, true },
    normal_assistant    = { r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#, false },
    unknown_error       = { r#"{"error":"Something unexpected happened"}"#, false },
    network_error       = { r#"{"error":"Network error occurred"}"#, false },
    empty_content       = { "", false },
    working_then_rate_limit = { "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{}}]}}\n{\"error\":\"Rate limit exceeded\"}\n", true },
    user_msg_before_error   = { "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n{\"error\":\"Too many requests\"}\n", true },
    old_error_then_working  = { "{\"error\":\"Rate limit exceeded\"}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Continuing work...\"}]}}\n", false },
)]
fn unrecoverable_error_detection(content: &str, expected: bool) {
    let (_dir, path) = temp_transcript(content);
    assert_eq!(has_unrecoverable_error(&path), expected);
}

#[test]
fn unrecoverable_error_missing_file() {
    assert!(!has_unrecoverable_error(std::path::Path::new(
        "/nonexistent/session.jsonl"
    )));
}
