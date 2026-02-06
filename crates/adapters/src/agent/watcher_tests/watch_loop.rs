// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::JobId;

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

    let params = log_watch_params(
        log_path.clone(),
        sessions,
        event_tx,
        shutdown_rx,
        Some(file_rx),
    );

    let handle = tokio::spawn(watch_loop(params));

    // Yield to let watch_loop read initial state before test modifies the file.
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

/// Helper to construct WatchLoopParams for fallback polling tests (no file watcher).
fn fallback_params(
    sessions: FakeSessionAdapter,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
) -> WatchLoopParams<FakeSessionAdapter> {
    WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-session".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::new("test-job")),
        log_path: None,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx: None,
    }
}

/// Helper to construct WatchLoopParams for tests with a log file.
fn log_watch_params(
    log_path: PathBuf,
    sessions: FakeSessionAdapter,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
    file_rx: Option<mpsc::Receiver<()>>,
) -> WatchLoopParams<FakeSessionAdapter> {
    WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::new("test-job")),
        log_path: Some(log_path),
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    }
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

// --- Fallback polling (watch_loop with no file watcher) Tests ---

#[tokio::test]
#[serial_test::serial]
async fn fallback_poll_exits_when_session_dies() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(watch_loop(fallback_params(sessions, event_tx, shutdown_rx)));

    tokio::time::sleep(Duration::from_millis(20)).await;
    sessions_clone.set_exited("test-session", 0);
    tokio::time::sleep(Duration::from_millis(30)).await;

    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentGone { .. })),
        "expected AgentGone event, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
    let _ = handle.await;
}

#[tokio::test]
#[serial_test::serial]
async fn fallback_poll_exits_on_shutdown() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let handle = tokio::spawn(watch_loop(fallback_params(sessions, event_tx, shutdown_rx)));

    tokio::time::sleep(Duration::from_millis(10)).await;
    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "should exit after shutdown signal");
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

    let params = log_watch_params(log_path, sessions, event_tx, shutdown_rx, Some(file_rx));

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

    let params = log_watch_params(log_path, sessions, event_tx, shutdown_rx, Some(file_rx));

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

    let params = log_watch_params(log_path, sessions, event_tx, shutdown_rx, Some(file_rx));

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

    let mut params = log_watch_params(
        log_path.clone(),
        sessions,
        event_tx,
        shutdown_rx,
        Some(file_rx),
    );
    params.log_entry_tx = Some(log_entry_tx);

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
    let params = log_watch_params(log_path, sessions, event_tx, shutdown_rx, Some(file_rx));

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

    let params = log_watch_params(log_path, sessions, event_tx, shutdown_rx, Some(file_rx));

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

// --- Fallback polling detects process exit (not session death) ---

#[tokio::test]
#[serial_test::serial]
async fn fallback_poll_detects_process_exit() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (_shutdown_tx, shutdown_rx) = oneshot::channel();
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(watch_loop(fallback_params(sessions, event_tx, shutdown_rx)));

    tokio::time::sleep(Duration::from_millis(20)).await;
    sessions_clone.set_process_running("test-session", false);
    tokio::time::sleep(Duration::from_millis(30)).await;

    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentExited { .. })),
        "expected AgentExited when process exits but session alive, got {:?}",
        event
    );

    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "fallback poll should exit");
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

    let mut params = log_watch_params(
        log_path.clone(),
        sessions,
        event_tx,
        shutdown_rx,
        Some(file_rx),
    );
    params.log_entry_tx = Some(log_entry_tx);

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
