// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

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
    yield_to_task().await;

    (event_rx, file_tx, shutdown_tx, log_path, handle)
}

/// Wait for the watch_loop to process pending messages and run a poll cycle.
async fn wait_for_poll() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
#[serial_test::serial]
async fn watcher_emits_agent_idle_for_waiting_state() {
    let (mut event_rx, file_tx, shutdown_tx, log_path, _handle) = setup_watch_loop().await;

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
    let (mut event_rx, file_tx, shutdown_tx, log_path, _handle) = setup_watch_loop().await;

    // Transition to Failed first
    append_line(&log_path, r#"{"error":"Rate limit exceeded"}"#);
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;
    let _ = event_rx.try_recv(); // drain AgentFailed

    // Transition back to Working
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
    tokio::time::sleep(Duration::from_millis(50)).await;

    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentIdle { .. })),
        "expected AgentIdle for initial WaitingForInput state, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
    let _ = file_tx;
}

#[tokio::test]
#[serial_test::serial]
async fn watcher_emits_event_for_initial_non_working_state() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.keep();
    let log_path = dir_path.join("session.jsonl");
    std::fs::write(&log_path, "{\"error\":\"Rate limit exceeded\"}\n").unwrap();

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-tmux", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let params = log_watch_params(log_path, sessions, event_tx, shutdown_rx, Some(file_rx));
    let _handle = tokio::spawn(watch_loop(params));
    tokio::time::sleep(Duration::from_millis(50)).await;

    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentFailed { .. })),
        "expected AgentFailed for initial failed state, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
    let _ = file_tx;
}

#[tokio::test]
#[serial_test::serial]
async fn watcher_does_not_emit_for_initial_working_state() {
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
    let (file_tx, file_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    std::env::set_var("OJ_WATCHER_POLL_MS", "50");

    let params = log_watch_params(log_path, sessions, event_tx, shutdown_rx, Some(file_rx));
    let _handle = tokio::spawn(watch_loop(params));
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        event_rx.try_recv().is_err(),
        "should not emit event for initial Working state"
    );

    let _ = shutdown_tx.send(());
    let _ = file_tx;
}

// --- watch_loop with log_entry_tx Tests ---

#[tokio::test]
#[serial_test::serial]
async fn watcher_extracts_log_entries_when_log_entry_tx_set() {
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

    let mut params = log_watch_params(log_path.clone(), sessions, event_tx, shutdown_rx, Some(file_rx));
    params.log_entry_tx = Some(log_entry_tx);

    let _handle = tokio::spawn(watch_loop(params));
    yield_to_task().await;

    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}]}}"#,
    );
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    let entry = log_entry_rx.try_recv();
    assert!(entry.is_ok(), "should receive log entries when log_entry_tx is set");
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
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions_clone = sessions.clone();
    let params = log_watch_params(log_path, sessions, event_tx, shutdown_rx, Some(file_rx));
    let handle = tokio::spawn(watch_loop(params));

    tokio::time::sleep(Duration::from_millis(5)).await;
    sessions_clone.set_exited("test-tmux", 0);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let event = event_rx.try_recv();
    assert!(
        matches!(event, Ok(Event::AgentGone { .. })),
        "expected AgentGone from liveness check, got {:?}",
        event
    );

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
    yield_to_task().await;

    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "watch_loop should exit after shutdown signal");
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

    // Send another file change notification with same state
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    assert!(
        event_rx.try_recv().is_err(),
        "should not emit duplicate event for same state"
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

    let mut params = log_watch_params(log_path.clone(), sessions, event_tx, shutdown_rx, Some(file_rx));
    params.log_entry_tx = Some(log_entry_tx);

    let _handle = tokio::spawn(watch_loop(params));
    yield_to_task().await;

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
