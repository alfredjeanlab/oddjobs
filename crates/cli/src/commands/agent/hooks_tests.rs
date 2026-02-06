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
