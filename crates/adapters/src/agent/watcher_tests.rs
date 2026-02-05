// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::session::{FakeSessionAdapter, SessionCall};
use std::io::Write;
use tempfile::TempDir;

/// Append a JSONL line to a file (simulates real session log appends).
fn append_line(path: &Path, content: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{}", content).unwrap();
}

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

#[test]
fn find_session_log_requires_correct_workspace_path() {
    // Regression test: the watcher must receive the agent's actual working
    // directory (workspace/cwd), not the project root. Claude Code derives
    // its project directory name from the cwd, so using a different path
    // produces a different directory name and the log is never found.
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();

    let session_id = "test-session";

    // Create session log at the hash derived from workspace_dir
    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join(format!("{session_id}.jsonl")),
        r#"{"type":"user","message":{"content":"hello"}}"#,
    )
    .unwrap();

    // Using the workspace path (correct) finds the log
    assert!(
        find_session_log_in(workspace_dir.path(), session_id, claude_base.path()).is_some(),
        "should find session log when given the workspace path"
    );

    // Using the project root (wrong) does NOT find the log
    assert!(
        find_session_log_in(project_dir.path(), session_id, claude_base.path()).is_none(),
        "should not find session log when given project_root (different hash)"
    );
}

/// Helper to set up the watcher loop for testing.
///
/// Returns (event_rx, file_tx, shutdown_tx, log_path) so the test can
/// drive file changes and observe emitted events.
async fn setup_watch_loop() -> (
    mpsc::Receiver<Event>,
    mpsc::Sender<()>,
    oneshot::Sender<()>,
    PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let dir = TempDir::new().unwrap();
    // Leak the TempDir so it lives for the test duration
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");

    // Start with a working state (trailing newline matches real JSONL format)
    std::fs::write(
        &log_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, event_rx) = mpsc::channel(32);
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Use a short poll interval so tests don't wait long
    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        log_path: log_path.clone(),
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

    let handle = tokio::spawn(watch_loop(params));

    // Yield to let watch_loop read initial state before test modifies the file.
    // The task must enter the select! loop before we write new content.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    (event_rx, file_tx, shutdown_tx, log_path, handle)
}

/// Wait for the watch_loop to process pending messages and run a poll cycle.
/// Uses a short real sleep since the poll interval is 50ms.
async fn wait_for_poll() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
#[serial_test::serial]
async fn watcher_emits_agent_idle_for_waiting_state() {
    // When the log shows WaitingForInput, the watcher emits AgentIdle (the same
    // event the Notification hook produces) instead of AgentWaiting. This provides
    // instant idle detection without the old timeout delay.
    let (mut event_rx, file_tx, shutdown_tx, log_path, _handle) = setup_watch_loop().await;

    // Append an idle state (text only, no thinking/tool_use)
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    );
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    let event = event_rx
        .try_recv()
        .expect("should emit AgentIdle when log shows waiting state");
    assert!(
        matches!(event, Event::AgentIdle { .. }),
        "expected AgentIdle (not AgentWaiting), got {event:?}"
    );

    let _ = shutdown_tx.send(());
}

#[tokio::test]
#[serial_test::serial]
async fn watcher_emits_working_to_failed_transition() {
    let (mut event_rx, file_tx, shutdown_tx, log_path, _handle) = setup_watch_loop().await;

    // Append error to transition to Failed
    append_line(&log_path, r#"{"error":"Rate limit exceeded"}"#);
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    let event = event_rx
        .try_recv()
        .expect("should emit AgentFailed on error");
    assert!(
        matches!(event, Event::AgentFailed { .. }),
        "expected AgentFailed, got {event:?}"
    );

    let _ = shutdown_tx.send(());
}

#[tokio::test]
#[serial_test::serial]
async fn watcher_emits_working_state_on_state_change() {
    // First go to a non-working state (failed), then back to working
    let (mut event_rx, file_tx, shutdown_tx, log_path, _handle) = setup_watch_loop().await;

    // Append error to transition to Failed first
    append_line(&log_path, r#"{"error":"Rate limit exceeded"}"#);
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;
    let _ = event_rx.try_recv(); // drain AgentFailed

    // Append user message to transition back to Working
    append_line(
        &log_path,
        r#"{"type":"user","message":{"content":"retry"}}"#,
    );
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    let event = event_rx
        .try_recv()
        .expect("should emit AgentWorking on recovery");
    assert!(
        matches!(event, Event::AgentWorking { .. }),
        "expected AgentWorking, got {event:?}"
    );

    let _ = shutdown_tx.send(());
}

// --- SessionLogParser incremental tests ---

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

// --- check_liveness Tests ---

#[tokio::test]
async fn check_liveness_returns_none_when_alive() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let agent_id = AgentId::new("test-agent");
    let result = check_liveness(&sessions, "test-session", "claude", &agent_id).await;

    assert!(
        result.is_none(),
        "should return None when session and process are alive"
    );
}

#[tokio::test]
async fn check_liveness_returns_session_gone_when_not_alive() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", false);

    let agent_id = AgentId::new("test-agent");
    let result = check_liveness(&sessions, "test-session", "claude", &agent_id).await;

    assert_eq!(result, Some(AgentState::SessionGone));
}

#[tokio::test]
async fn check_liveness_returns_session_gone_for_missing_session() {
    let sessions = FakeSessionAdapter::new();
    // Don't add any session - is_alive will return false

    let agent_id = AgentId::new("test-agent");
    let result = check_liveness(&sessions, "nonexistent", "claude", &agent_id).await;

    assert_eq!(result, Some(AgentState::SessionGone));
}

#[tokio::test]
async fn check_liveness_returns_exited_when_process_not_running() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    // Session is alive but process has exited - this is the case where
    // tmux is still running but the claude process inside has terminated.
    // Note: don't call set_exited as it sets alive=false
    sessions.set_process_running("test-session", false);

    let agent_id = AgentId::new("test-agent");
    let result = check_liveness(&sessions, "test-session", "claude", &agent_id).await;

    // Exit code will be None since we didn't set it (and can't without setting alive=false)
    assert!(
        matches!(result, Some(AgentState::Exited { exit_code: None })),
        "expected Exited with exit_code None, got {:?}",
        result
    );
}

// --- check_and_accept_trust_prompt Tests ---

#[tokio::test]
async fn check_trust_prompt_detected_and_accepted() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_output(
        "test-session",
        vec!["Do you trust the files in this folder?".to_string()],
    );

    let result = check_and_accept_trust_prompt(&sessions, "test-session").await;

    assert!(result, "should detect and accept trust prompt");

    // Verify that "y" was sent
    let calls = sessions.calls();
    let send_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, SessionCall::Send { .. }))
        .collect();
    assert!(
        send_calls.iter().any(|c| matches!(
            c,
            SessionCall::Send { input, .. } if input == "y"
        )),
        "should send 'y' to accept trust prompt"
    );
}

#[tokio::test]
async fn check_trust_prompt_short_pattern_detected() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_output("test-session", vec!["Do you trust".to_string()]);

    let result = check_and_accept_trust_prompt(&sessions, "test-session").await;

    assert!(result, "should detect short trust pattern");
}

#[tokio::test]
async fn check_trust_prompt_not_present() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_output(
        "test-session",
        vec![
            "Welcome to Claude".to_string(),
            "How can I help?".to_string(),
        ],
    );

    let result = check_and_accept_trust_prompt(&sessions, "test-session").await;

    assert!(!result, "should return false when no trust prompt");
}

#[tokio::test]
async fn check_trust_prompt_capture_error_returns_false() {
    let sessions = FakeSessionAdapter::new();
    // Don't add session - capture_output will fail

    let result = check_and_accept_trust_prompt(&sessions, "nonexistent").await;

    assert!(!result, "should return false on capture error");
}

// --- poll_process_only Tests ---

#[tokio::test]
#[serial_test::serial]
async fn poll_process_only_exits_when_session_dies() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let agent_id = AgentId::new("test-agent");
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Use very short poll interval
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(poll_process_only(
        agent_id,
        "test-session".to_string(),
        "claude".to_string(),
        sessions,
        event_tx,
        shutdown_rx,
    ));

    // Let it poll once successfully
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Now kill the session - set_exited sets alive=false, so we'll get SessionGone
    sessions_clone.set_exited("test-session", 0);

    // Wait for the poll to detect it
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Should receive a SessionGone event (because set_exited sets alive=false)
    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentGone { .. })),
        "expected AgentGone event, got {:?}",
        event
    );

    // Clean up
    let _ = shutdown_tx.send(());
    let _ = handle.await;
}

#[tokio::test]
#[serial_test::serial]
async fn poll_process_only_exits_on_shutdown() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let agent_id = AgentId::new("test-agent");
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let handle = tokio::spawn(poll_process_only(
        agent_id,
        "test-session".to_string(),
        "claude".to_string(),
        sessions,
        event_tx,
        shutdown_rx,
    ));

    // Let it start
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Send shutdown
    shutdown_tx.send(()).unwrap();

    // Should exit without emitting events
    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "should exit after shutdown signal");

    // No events should have been emitted
    assert!(
        event_rx.try_recv().is_err(),
        "should not emit events on clean shutdown"
    );
}

// --- Initial State Detection Tests ---

#[tokio::test]
#[serial_test::serial]
async fn watcher_emits_idle_immediately_for_initial_waiting_state() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");

    // Start with WaitingForInput state (text only, no tool_use)
    std::fs::write(
        &log_path,
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Done!\"}]}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

    let _handle = tokio::spawn(watch_loop(params));

    // Wait briefly for initial state to be emitted
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should receive AgentIdle immediately for initial WaitingForInput state
    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentIdle { .. })),
        "expected AgentIdle for initial WaitingForInput state, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
    let _ = file_tx; // silence unused warning
}

#[tokio::test]
#[serial_test::serial]
async fn watcher_emits_event_for_initial_non_working_state() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");

    // Start with Failed state
    std::fs::write(&log_path, "{\"error\":\"Rate limit exceeded\"}\n").unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

    let _handle = tokio::spawn(watch_loop(params));

    // Wait briefly for initial state to be emitted
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should receive AgentFailed immediately for initial failed state
    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentFailed { .. })),
        "expected AgentFailed for initial failed state, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
    let _ = file_tx; // silence unused warning
}

#[tokio::test]
#[serial_test::serial]
async fn watcher_does_not_emit_for_initial_working_state() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");

    // Start with Working state (user message)
    std::fs::write(
        &log_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

    let _handle = tokio::spawn(watch_loop(params));

    // Wait briefly
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should NOT receive any event for initial Working state
    let event = event_rx.try_recv();
    assert!(
        event.is_err(),
        "should not emit event for initial Working state, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
    let _ = file_tx; // silence unused warning
}

// --- find_session_log Tests ---

#[test]
fn find_session_log_in_uses_fallback_for_missing_session() {
    // When session file doesn't exist, it falls back to most recent .jsonl
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    // Create project directory
    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();

    // Create a different session file
    let other_session_path = log_dir.join("other-session.jsonl");
    std::fs::write(&other_session_path, r#"{"type":"user"}"#).unwrap();

    // Look for non-existent session - should fall back to other-session.jsonl
    let result = find_session_log_in(
        workspace_dir.path(),
        "nonexistent-session",
        claude_base.path(),
    );

    assert!(result.is_some());
    assert_eq!(result.unwrap(), other_session_path);
}

#[test]
fn find_session_log_in_returns_none_for_missing_project() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    // Don't create any project directory

    let result = find_session_log_in(workspace_dir.path(), "any-session", claude_base.path());

    assert!(result.is_none());
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

// --- wait_for_session_log_or_exit Tests ---

#[tokio::test]
#[serial_test::serial]
async fn wait_for_session_log_found_immediately() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let session_id = "test-session-found";

    // Create session log at the expected location
    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join(format!("{session_id}.jsonl")),
        r#"{"type":"user","message":{"content":"hello"}}"#,
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("tmux-session", true);

    let result =
        wait_for_session_log_or_exit(workspace_dir.path(), session_id, "tmux-session", &sessions)
            .await;

    assert!(
        matches!(result, SessionLogWait::Found(_)),
        "expected Found, got {:?}",
        std::mem::discriminant(&result)
    );

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
}

#[tokio::test]
#[serial_test::serial]
async fn wait_for_session_log_session_died() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let sessions = FakeSessionAdapter::new();
    // Session is dead from the start
    sessions.add_session("dead-tmux", false);

    let result = wait_for_session_log_or_exit(
        workspace_dir.path(),
        "nonexistent-session",
        "dead-tmux",
        &sessions,
    )
    .await;

    assert!(
        matches!(result, SessionLogWait::SessionDied),
        "expected SessionDied"
    );

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
}

#[tokio::test]
#[serial_test::serial]
async fn wait_for_session_log_timeout() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let sessions = FakeSessionAdapter::new();
    // Session alive but log never created → timeout after 30 iterations
    sessions.add_session("alive-tmux", true);

    let result = wait_for_session_log_or_exit(
        workspace_dir.path(),
        "never-created-session",
        "alive-tmux",
        &sessions,
    )
    .await;

    assert!(
        matches!(result, SessionLogWait::Timeout),
        "expected Timeout"
    );

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
}

#[tokio::test]
#[serial_test::serial]
async fn wait_for_session_log_checks_trust_prompt_early() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("trust-tmux", true);
    sessions.set_output(
        "trust-tmux",
        vec!["Do you trust the files in this folder?".to_string()],
    );

    // Log never appears, so this will timeout, but trust prompt should be checked
    let _ =
        wait_for_session_log_or_exit(workspace_dir.path(), "no-session", "trust-tmux", &sessions)
            .await;

    // Verify trust prompt was detected and "y" was sent
    let calls = sessions.calls();
    let send_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, SessionCall::Send { input, .. } if input == "y"))
        .collect();
    assert!(
        !send_calls.is_empty(),
        "should send 'y' for trust prompt during early iterations"
    );

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
}

// --- watch_loop with log_entry_tx Tests ---

#[tokio::test]
#[serial_test::serial]
async fn watcher_extracts_log_entries_when_log_entry_tx_set() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");

    // Start with a user message
    std::fs::write(
        &log_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, _event_rx) = mpsc::channel(32);
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (log_entry_tx, mut log_entry_rx) = mpsc::channel(32);

    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        log_path: log_path.clone(),
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: Some(log_entry_tx),
        file_rx,
    };

    let _handle = tokio::spawn(watch_loop(params));

    // Yield to let watch_loop initialize
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Append an assistant message with a tool_use (Bash command) - this creates a log entry
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}]}}"#,
    );
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    // Check that log entries were forwarded
    let entry = log_entry_rx.try_recv();
    assert!(
        entry.is_ok(),
        "should receive log entries when log_entry_tx is set"
    );
    let (agent_id, entries) = entry.unwrap();
    assert_eq!(agent_id, AgentId::new("test-agent"));
    assert!(!entries.is_empty(), "should have extracted log entries");

    let _ = shutdown_tx.send(());
}

// --- watch_loop liveness check breaks loop ---

#[tokio::test]
#[serial_test::serial]
async fn watcher_detects_process_death_via_liveness_check() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");

    // Start with working state
    std::fs::write(
        &log_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (_file_tx, file_rx) = mpsc::channel(32);
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();

    // Very short poll interval so liveness check fires quickly
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions_clone = sessions.clone();
    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

    let handle = tokio::spawn(watch_loop(params));

    // Let the loop start
    tokio::time::sleep(Duration::from_millis(5)).await;

    // Kill the session
    sessions_clone.set_exited("test-tmux", 0);

    // Wait for liveness check to detect it
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should receive AgentGone and the loop should have exited
    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentGone { .. })),
        "expected AgentGone from liveness check, got {:?}",
        event
    );

    // The handle should complete since the loop broke
    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "watch_loop should exit after session dies");
}

// --- watch_loop shutdown breaks loop ---

#[tokio::test]
#[serial_test::serial]
async fn watcher_exits_on_shutdown_signal() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");

    std::fs::write(
        &log_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (_file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    std::env::set_var("OJ_WATCHER_POLL_MS", "5000");

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

    let handle = tokio::spawn(watch_loop(params));

    // Yield to let the loop start
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Send shutdown
    shutdown_tx.send(()).unwrap();

    // Should exit promptly
    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(
        result.is_ok(),
        "watch_loop should exit after shutdown signal"
    );

    // No agent-death events should have been emitted
    assert!(
        event_rx.try_recv().is_err(),
        "should not emit events on clean shutdown"
    );
}

// --- watch_loop no duplicate events for same state ---

#[tokio::test]
#[serial_test::serial]
async fn watcher_does_not_emit_duplicate_state_changes() {
    let (mut event_rx, file_tx, shutdown_tx, log_path, _handle) = setup_watch_loop().await;

    // Append idle state
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    );
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentIdle { .. })),
        "first idle event should be emitted"
    );

    // Send another file change notification with same state (no new log content)
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    // Should NOT emit a duplicate event
    let event = event_rx.try_recv();
    assert!(
        event.is_err(),
        "should not emit duplicate event for same state, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
}

// --- watch_agent full lifecycle tests ---

#[tokio::test]
#[serial_test::serial]
async fn watch_agent_emits_agent_gone_when_session_dies_before_log() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("dead-session", false);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let config = WatcherConfig {
        agent_id: AgentId::new("test-agent"),
        log_session_id: "nonexistent-log".to_string(),
        tmux_session_id: "dead-session".to_string(),
        project_path: workspace_dir.path().to_path_buf(),
        process_name: "claude".to_string(),
    };

    let handle = tokio::spawn(watch_agent(config, sessions, event_tx, shutdown_rx, None));

    // Wait for event
    let event = tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await;
    assert!(
        matches!(event, Ok(Some(Event::AgentGone { .. }))),
        "expected AgentGone when session dies before log, got {:?}",
        event
    );

    // watch_agent should exit
    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "watch_agent should exit after AgentGone");

    let _ = shutdown_tx.send(());
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}

#[tokio::test]
#[serial_test::serial]
async fn watch_agent_falls_back_to_poll_on_timeout() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("alive-session", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let config = WatcherConfig {
        agent_id: AgentId::new("test-agent"),
        log_session_id: "never-created".to_string(),
        tmux_session_id: "alive-session".to_string(),
        project_path: workspace_dir.path().to_path_buf(),
        process_name: "claude".to_string(),
    };

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(watch_agent(config, sessions, event_tx, shutdown_rx, None));

    // Wait for timeout (30 iterations * 1ms = ~30ms), then fallback polling starts
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now kill the session during fallback polling
    sessions_clone.set_exited("alive-session", 1);

    // Should detect via fallback polling
    let event = tokio::time::timeout(Duration::from_millis(200), event_rx.recv()).await;
    assert!(
        matches!(event, Ok(Some(Event::AgentGone { .. }))),
        "expected AgentGone during fallback polling, got {:?}",
        event
    );

    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "watch_agent should exit after session dies");

    let _ = shutdown_tx.send(());
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}

#[tokio::test]
#[serial_test::serial]
async fn watch_agent_with_session_log_enters_watch_loop() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let session_id = "found-session";

    // Create the session log at the expected location
    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    let log_file = log_dir.join(format!("{session_id}.jsonl"));
    std::fs::write(
        &log_file,
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("tmux-found", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let config = WatcherConfig {
        agent_id: AgentId::new("test-agent"),
        log_session_id: session_id.to_string(),
        tmux_session_id: "tmux-found".to_string(),
        project_path: workspace_dir.path().to_path_buf(),
        process_name: "claude".to_string(),
    };

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(watch_agent(config, sessions, event_tx, shutdown_rx, None));

    // Let it enter watch_loop (log found → watch_loop starts)
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Initial state is Working so no event emitted yet
    assert!(
        event_rx.try_recv().is_err(),
        "no event for initial Working state"
    );

    // Kill the session to trigger liveness check
    sessions_clone.set_exited("tmux-found", 0);

    // Wait for poll to detect
    tokio::time::sleep(Duration::from_millis(50)).await;

    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentGone { .. })),
        "expected AgentGone from watch_loop liveness check, got {:?}",
        event
    );

    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "watch_agent should exit");

    let _ = shutdown_tx.send(());
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}

// --- start_watcher Tests ---

#[tokio::test]
#[serial_test::serial]
async fn start_watcher_returns_shutdown_sender() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("start-watcher-tmux", false);

    let (event_tx, mut event_rx) = mpsc::channel(32);

    let config = WatcherConfig {
        agent_id: AgentId::new("test-agent"),
        log_session_id: "start-watcher-session".to_string(),
        tmux_session_id: "start-watcher-tmux".to_string(),
        project_path: workspace_dir.path().to_path_buf(),
        process_name: "claude".to_string(),
    };

    let shutdown_tx = start_watcher(config, sessions, event_tx, None);

    // Session is dead, should emit AgentGone
    let event = tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await;
    assert!(
        matches!(event, Ok(Some(Event::AgentGone { .. }))),
        "expected AgentGone from start_watcher, got {:?}",
        event
    );

    // Shutdown sender should be usable (even if the task already completed)
    let _ = shutdown_tx.send(());

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}

#[tokio::test]
#[serial_test::serial]
async fn start_watcher_shutdown_stops_watcher() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1000");
    std::env::set_var("OJ_WATCHER_POLL_MS", "5000");

    let session_id = "shutdown-session";

    // Create the session log so it enters watch_loop
    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join(format!("{session_id}.jsonl")),
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("shutdown-tmux", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);

    let config = WatcherConfig {
        agent_id: AgentId::new("test-agent"),
        log_session_id: session_id.to_string(),
        tmux_session_id: "shutdown-tmux".to_string(),
        project_path: workspace_dir.path().to_path_buf(),
        process_name: "claude".to_string(),
    };

    let shutdown_tx = start_watcher(config, sessions, event_tx, None);

    // Let it start and enter watch_loop
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send shutdown
    shutdown_tx.send(()).unwrap();

    // Give it time to process
    tokio::time::sleep(Duration::from_millis(50)).await;

    // No death events
    assert!(
        event_rx.try_recv().is_err(),
        "no events after clean shutdown"
    );

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}

// --- find_session_log with CLAUDE_CONFIG_DIR Tests ---

#[test]
#[serial_test::serial]
fn find_session_log_uses_claude_config_dir_env() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());

    let session_id = "env-var-session";
    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    let session_file = log_dir.join(format!("{session_id}.jsonl"));
    std::fs::write(&session_file, r#"{"type":"user"}"#).unwrap();

    let result = find_session_log(workspace_dir.path(), session_id);
    assert!(
        result.is_some(),
        "should find session log via CLAUDE_CONFIG_DIR"
    );
    assert_eq!(result.unwrap(), session_file);

    std::env::remove_var("CLAUDE_CONFIG_DIR");
}

#[test]
#[serial_test::serial]
fn find_session_log_returns_none_when_no_log_exists() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());

    let result = find_session_log(workspace_dir.path(), "nonexistent");
    assert!(result.is_none());

    std::env::remove_var("CLAUDE_CONFIG_DIR");
}

// --- check_liveness edge cases ---

#[tokio::test]
async fn check_liveness_returns_exited_with_exit_code() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", false);

    let agent_id = AgentId::new("test-agent");
    let result = check_liveness(&sessions, "test-session", "claude", &agent_id).await;

    assert!(
        matches!(result, Some(AgentState::Exited { exit_code: None })),
        "expected Exited with no exit code, got {:?}",
        result
    );
}

// --- poll_process_only with process exit (not session death) ---

#[tokio::test]
#[serial_test::serial]
async fn poll_process_only_detects_process_exit() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let agent_id = AgentId::new("test-agent");
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();

    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(poll_process_only(
        agent_id,
        "test-session".to_string(),
        "claude".to_string(),
        sessions,
        event_tx,
        shutdown_rx,
    ));

    // Let it poll once
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Process exits but tmux session still alive
    sessions_clone.set_process_running("test-session", false);

    // Wait for poll to detect
    tokio::time::sleep(Duration::from_millis(30)).await;

    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentExited { .. })),
        "expected AgentExited when process exits but session alive, got {:?}",
        event
    );

    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "poll_process_only should exit");
}

// --- watch_loop with log_entry_tx extraction on state change ---

#[tokio::test]
#[serial_test::serial]
async fn watcher_forwards_log_entries_on_file_change() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");

    std::fs::write(
        &log_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, _event_rx) = mpsc::channel(32);
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (log_entry_tx, mut log_entry_rx) = mpsc::channel(32);

    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        log_path: log_path.clone(),
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: Some(log_entry_tx),
        file_rx,
    };

    let _handle = tokio::spawn(watch_loop(params));

    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    // Append a Read tool use entry (will produce a FileRead log entry)
    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/tmp/test.txt"}}]}}"#,
    );
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    let entry = log_entry_rx.try_recv();
    assert!(entry.is_ok(), "should forward log entries");
    let (agent_id, entries) = entry.unwrap();
    assert_eq!(agent_id, AgentId::new("test-agent"));
    assert!(
        entries.iter().any(|e| matches!(&e.kind, log_entry::EntryKind::FileRead { path } if path == "/tmp/test.txt")),
        "should contain FileRead entry, got {:?}",
        entries
    );

    let _ = shutdown_tx.send(());
}

// --- start_watcher with log_entry_tx ---

#[tokio::test]
#[serial_test::serial]
async fn start_watcher_with_log_entry_tx() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("log-entry-tmux", false);

    let (event_tx, _event_rx) = mpsc::channel(32);
    let (log_entry_tx, _log_entry_rx) = mpsc::channel(32);

    let config = WatcherConfig {
        agent_id: AgentId::new("test-agent"),
        log_session_id: "log-entry-session".to_string(),
        tmux_session_id: "log-entry-tmux".to_string(),
        project_path: workspace_dir.path().to_path_buf(),
        process_name: "claude".to_string(),
    };

    // Verify start_watcher accepts log_entry_tx
    let shutdown_tx = start_watcher(config, sessions, event_tx, Some(log_entry_tx));

    // Clean up - session is dead so it exits quickly
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = shutdown_tx.send(());

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}

// --- find_session_log_in fallback to most recent file ---

#[test]
fn find_session_log_in_picks_most_recent_fallback() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();

    // Create two session files with different modification times
    let older = log_dir.join("older-session.jsonl");
    std::fs::write(&older, r#"{"type":"user"}"#).unwrap();

    // Force a small time gap
    std::thread::sleep(Duration::from_millis(50));

    let newer = log_dir.join("newer-session.jsonl");
    std::fs::write(&newer, r#"{"type":"user"}"#).unwrap();

    // Look for non-existent session - should fall back to most recent (newer)
    let result = find_session_log_in(
        workspace_dir.path(),
        "nonexistent-session",
        claude_base.path(),
    );

    assert!(result.is_some());
    assert_eq!(
        result.unwrap(),
        newer,
        "should fall back to most recently modified file"
    );
}

// --- find_session_log_in with empty project directory ---

#[test]
fn find_session_log_in_returns_none_for_empty_project_dir() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    // Directory exists but no .jsonl files

    let result = find_session_log_in(workspace_dir.path(), "any-session", claude_base.path());

    assert!(
        result.is_none(),
        "should return None when project dir exists but has no jsonl files"
    );
}
