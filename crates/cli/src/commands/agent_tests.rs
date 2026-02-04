// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use serial_test::serial;
use tempfile::tempdir;

#[test]
fn utc_timestamp_format_is_valid() {
    let ts = utc_timestamp();
    // Should match YYYY-MM-DDTHH:MM:SSZ
    assert_eq!(ts.len(), 20, "timestamp length should be 20: {ts}");
    assert!(ts.ends_with('Z'), "timestamp should end with Z: {ts}");
    assert_eq!(&ts[4..5], "-");
    assert_eq!(&ts[7..8], "-");
    assert_eq!(&ts[10..11], "T");
    assert_eq!(&ts[13..14], ":");
    assert_eq!(&ts[16..17], ":");
}

#[test]
#[serial]
fn get_state_dir_respects_env_var() {
    let dir = tempdir().unwrap();
    let prev = std::env::var("OJ_STATE_DIR").ok();
    std::env::set_var("OJ_STATE_DIR", dir.path());
    let result = get_state_dir();
    match prev {
        Some(v) => std::env::set_var("OJ_STATE_DIR", v),
        None => std::env::remove_var("OJ_STATE_DIR"),
    }
    assert_eq!(result, dir.path());
}

#[test]
#[serial]
fn append_agent_log_writes_to_file() {
    let dir = tempdir().unwrap();
    let prev = std::env::var("OJ_STATE_DIR").ok();
    std::env::set_var("OJ_STATE_DIR", dir.path());

    append_agent_log("test-agent-123", "invoked, stop_hook_active=false");
    append_agent_log("test-agent-123", "blocking exit, signaled=false");

    match prev {
        Some(v) => std::env::set_var("OJ_STATE_DIR", v),
        None => std::env::remove_var("OJ_STATE_DIR"),
    }

    let log_path = dir.path().join("logs/agent/test-agent-123.log");
    assert!(log_path.exists(), "log file should be created");
    let content = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "should have 2 log lines");
    assert!(
        lines[0].contains("stop-hook: invoked, stop_hook_active=false"),
        "first line: {}",
        lines[0]
    );
    assert!(
        lines[1].contains("stop-hook: blocking exit, signaled=false"),
        "second line: {}",
        lines[1]
    );
    // Verify timestamp prefix
    assert!(lines[0].contains("T"), "should have timestamp");
    assert!(lines[0].contains("Z"), "should have UTC marker");
}

#[test]
fn prompt_type_for_tool_maps_exit_plan_mode() {
    assert_eq!(
        prompt_type_for_tool(Some("ExitPlanMode")),
        Some(oj_core::PromptType::PlanApproval)
    );
}

#[test]
fn prompt_type_for_tool_maps_enter_plan_mode() {
    assert_eq!(
        prompt_type_for_tool(Some("EnterPlanMode")),
        Some(oj_core::PromptType::PlanApproval)
    );
}

#[test]
fn prompt_type_for_tool_maps_ask_user_question() {
    assert_eq!(
        prompt_type_for_tool(Some("AskUserQuestion")),
        Some(oj_core::PromptType::Question)
    );
}

#[test]
fn prompt_type_for_tool_returns_none_for_unknown() {
    assert_eq!(prompt_type_for_tool(Some("Bash")), None);
    assert_eq!(prompt_type_for_tool(None), None);
}

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
