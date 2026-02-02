// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::session::FakeSessionAdapter;
use oj_core::FakeClock;
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

/// Helper to set up the watcher loop with FakeClock for idle timeout tests.
///
/// Returns (event_rx, file_tx, shutdown_tx, log_path, clock) so the test can
/// drive file changes, advance time, and observe emitted events.
async fn setup_watch_loop(
    idle_timeout: Duration,
) -> (
    mpsc::Receiver<Event>,
    mpsc::Sender<()>,
    oneshot::Sender<()>,
    PathBuf,
    FakeClock,
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

    let clock = FakeClock::new();
    let (event_tx, event_rx) = mpsc::channel(32);
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Use a short poll interval so tests don't wait long for the idle check
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
        clock: clock.clone(),
        file_rx,
        idle_timeout: Some(idle_timeout),
    };

    let handle = tokio::spawn(watch_loop(params));

    // Yield to let watch_loop read initial state before test modifies the file.
    // The task must enter the select! loop before we write new content.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    (event_rx, file_tx, shutdown_tx, log_path, clock, handle)
}

/// Wait for the watch_loop to process pending messages and run a poll cycle.
/// Uses a short real sleep since the poll interval is 50ms.
async fn wait_for_poll() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
#[serial_test::serial]
async fn idle_timeout_not_emitted_for_thinking_blocks() {
    // Use a large idle timeout (controlled by FakeClock, not real time)
    let (mut event_rx, file_tx, shutdown_tx, log_path, clock, _handle) =
        setup_watch_loop(Duration::from_secs(300)).await;

    // Write a thinking block — should be classified as Working
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"analyzing..."}]}}"#,
    )
    .unwrap();
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    // Advance FakeClock well past idle timeout
    clock.advance(Duration::from_secs(600));
    wait_for_poll().await;

    // No event should be emitted — thinking block means Working, not WaitingForInput
    assert!(
        event_rx.try_recv().is_err(),
        "thinking block should NOT trigger idle event"
    );

    let _ = shutdown_tx.send(());
}

#[tokio::test]
#[serial_test::serial]
async fn idle_timeout_respects_clock_before_emitting() {
    let (mut event_rx, file_tx, shutdown_tx, log_path, clock, _handle) =
        setup_watch_loop(Duration::from_secs(300)).await;

    // Transition to WaitingForInput (text only, no thinking/tool_use)
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    )
    .unwrap();
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    // No event yet — WaitingForInput should NOT be emitted immediately
    assert!(
        event_rx.try_recv().is_err(),
        "WaitingForInput should NOT be emitted immediately"
    );

    // Advance FakeClock but NOT past idle timeout
    clock.advance(Duration::from_secs(200));
    wait_for_poll().await;

    assert!(
        event_rx.try_recv().is_err(),
        "WaitingForInput should NOT be emitted before idle timeout (200s < 300s)"
    );

    // Advance FakeClock past idle timeout
    clock.advance(Duration::from_secs(200)); // total 400s > 300s
    wait_for_poll().await;

    let event = event_rx
        .try_recv()
        .expect("idle timeout should emit AgentWaiting");
    assert!(
        matches!(event, Event::AgentWaiting { .. }),
        "expected AgentWaiting, got {event:?}"
    );

    let _ = shutdown_tx.send(());
}

#[tokio::test]
#[serial_test::serial]
async fn idle_timeout_resets_when_agent_resumes_working() {
    let (mut event_rx, file_tx, shutdown_tx, log_path, clock, _handle) =
        setup_watch_loop(Duration::from_secs(300)).await;

    // Transition to WaitingForInput
    std::fs::write(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done!"}]}}"#,
    )
    .unwrap();
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    // Advance close to timeout
    clock.advance(Duration::from_secs(250));
    wait_for_poll().await;

    // No idle event yet (250s < 300s)
    assert!(event_rx.try_recv().is_err());

    // Agent resumes work (user message = tool result processing)
    std::fs::write(
        &log_path,
        r#"{"type":"user","message":{"content":"tool result"}}"#,
    )
    .unwrap();
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    // Drain the AgentWorking event from the Working transition
    let _ = event_rx.try_recv(); // AgentWorking from WaitingForInput→Working

    // Advance past the original timeout point — should NOT fire because timer was reset
    clock.advance(Duration::from_secs(100)); // 250 + 100 from original, but only 100 from reset
    wait_for_poll().await;

    assert!(
        event_rx.try_recv().is_err(),
        "idle timeout should have reset when agent resumed working"
    );

    let _ = shutdown_tx.send(());
}
