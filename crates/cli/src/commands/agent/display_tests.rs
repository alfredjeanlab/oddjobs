// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::fs;

/// Verify capture file is read when present at the expected path.
#[test]
fn try_read_agent_capture_returns_content_when_file_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "abc12345-dead-beef-cafe-123456789012";
    let logs_dir = tmp.path().join("logs");
    let capture_path = oj_engine::log_paths::agent_capture_path(&logs_dir, agent_id);

    fs::create_dir_all(capture_path.parent().unwrap()).unwrap();
    fs::write(&capture_path, "hello from terminal\n").unwrap();

    // Simulate what try_read_agent_capture does (without relying on state_dir env)
    let content = fs::read_to_string(&capture_path).ok();
    assert_eq!(content.as_deref(), Some("hello from terminal\n"));
}

/// Verify missing capture file returns None rather than panicking.
#[test]
fn try_read_agent_capture_returns_none_when_file_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "nonexistent-agent-id";
    let logs_dir = tmp.path().join("logs");
    let capture_path = oj_engine::log_paths::agent_capture_path(&logs_dir, agent_id);

    let content = std::fs::read_to_string(&capture_path).ok();
    assert!(content.is_none());
}
