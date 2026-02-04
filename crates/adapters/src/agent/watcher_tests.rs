// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::session::FakeSessionAdapter;
use tempfile::TempDir;

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

    // Start with a working state
    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"hello"}}"#,
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

    // Write an idle state (text only, no thinking/tool_use)
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    )
    .unwrap();
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

    // Transition to Failed
    std::fs::write(&log_path, r#"{"error":"Rate limit exceeded"}"#).unwrap();
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

    // Transition to Failed first
    std::fs::write(&log_path, r#"{"error":"Rate limit exceeded"}"#).unwrap();
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;
    let _ = event_rx.try_recv(); // drain AgentFailed

    // Transition back to Working
    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"retry"}}"#,
    )
    .unwrap();
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
