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

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::default()),
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

// --- State Transition Events ---

#[tokio::test]
#[serial_test::serial]
async fn emits_agent_idle_for_waiting_state() {
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
async fn emits_working_to_failed_transition() {
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
async fn emits_working_state_on_state_change() {
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

#[tokio::test]
#[serial_test::serial]
async fn does_not_emit_duplicate_state_changes() {
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

    // Send another file change notification with same state (no new log content)
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

    let event = event_rx.try_recv();
    assert!(
        event.is_err(),
        "should not emit duplicate event for same state, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
}

// --- Initial State Detection ---

#[tokio::test]
#[serial_test::serial]
async fn emits_idle_immediately_for_initial_waiting_state() {
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

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::default()),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

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
async fn emits_event_for_initial_non_working_state() {
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

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::default()),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

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
async fn does_not_emit_for_initial_working_state() {
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

    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::default()),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

    let _handle = tokio::spawn(watch_loop(params));

    tokio::time::sleep(Duration::from_millis(50)).await;

    let event = event_rx.try_recv();
    assert!(
        event.is_err(),
        "should not emit event for initial Working state, got {:?}",
        event
    );

    let _ = shutdown_tx.send(());
    let _ = file_tx;
}

// --- Liveness and Shutdown ---

#[tokio::test]
#[serial_test::serial]
async fn detects_process_death_via_liveness_check() {
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
    let params = WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::default()),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

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

#[tokio::test]
#[serial_test::serial]
async fn exits_on_shutdown_signal() {
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
        owner: OwnerId::Job(JobId::default()),
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    };

    let handle = tokio::spawn(watch_loop(params));

    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(
        result.is_ok(),
        "watch_loop should exit after shutdown signal"
    );

    assert!(
        event_rx.try_recv().is_err(),
        "should not emit events on clean shutdown"
    );
}

// --- Log Entry Forwarding ---

#[tokio::test]
#[serial_test::serial]
async fn extracts_log_entries_when_log_entry_tx_set() {
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
        owner: OwnerId::Job(JobId::default()),
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

    append_line(
        &log_path,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}]}}"#,
    );
    file_tx.send(()).await.unwrap();
    wait_for_poll().await;

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

#[tokio::test]
#[serial_test::serial]
async fn forwards_log_entries_on_file_change() {
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
        owner: OwnerId::Job(JobId::default()),
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
