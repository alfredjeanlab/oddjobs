// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::JobId;

// --- check_liveness ---

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

    let agent_id = AgentId::new("test-agent");
    let result = check_liveness(&sessions, "nonexistent", "claude", &agent_id).await;

    assert_eq!(result, Some(AgentState::SessionGone));
}

#[tokio::test]
async fn check_liveness_returns_exited_when_process_not_running() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", false);

    let agent_id = AgentId::new("test-agent");
    let result = check_liveness(&sessions, "test-session", "claude", &agent_id).await;

    assert!(
        matches!(result, Some(AgentState::Exited { exit_code: None })),
        "expected Exited with exit_code None, got {:?}",
        result
    );
}

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

// --- check_and_accept_trust_prompt ---

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

    let result = check_and_accept_trust_prompt(&sessions, "nonexistent").await;

    assert!(!result, "should return false on capture error");
}

// --- poll_process_only ---

#[tokio::test]
#[serial_test::serial]
async fn poll_process_only_exits_when_session_dies() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let agent_id = AgentId::new("test-agent");
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(poll_process_only(
        agent_id,
        "test-session".to_string(),
        "claude".to_string(),
        OwnerId::Job(JobId::default()),
        sessions,
        event_tx,
        shutdown_rx,
    ));

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
        OwnerId::Job(JobId::default()),
        sessions,
        event_tx,
        shutdown_rx,
    ));

    tokio::time::sleep(Duration::from_millis(10)).await;

    shutdown_tx.send(()).unwrap();

    let result = tokio::time::timeout(Duration::from_millis(200), handle).await;
    assert!(result.is_ok(), "should exit after shutdown signal");

    assert!(
        event_rx.try_recv().is_err(),
        "should not emit events on clean shutdown"
    );
}

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
        OwnerId::Job(JobId::default()),
        sessions,
        event_tx,
        shutdown_rx,
    ));

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
    assert!(result.is_ok(), "poll_process_only should exit");
}

// --- find_session_log ---

#[test]
fn find_session_log_in_uses_fallback_for_missing_session() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();

    let other_session_path = log_dir.join("other-session.jsonl");
    std::fs::write(&other_session_path, r#"{"type":"user"}"#).unwrap();

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

    let result = find_session_log_in(workspace_dir.path(), "any-session", claude_base.path());

    assert!(result.is_none());
}

#[test]
fn find_session_log_in_picks_most_recent_fallback() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();

    let older = log_dir.join("older-session.jsonl");
    std::fs::write(&older, r#"{"type":"user"}"#).unwrap();

    std::thread::sleep(Duration::from_millis(50));

    let newer = log_dir.join("newer-session.jsonl");
    std::fs::write(&newer, r#"{"type":"user"}"#).unwrap();

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

#[test]
fn find_session_log_in_returns_none_for_empty_project_dir() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();

    let result = find_session_log_in(workspace_dir.path(), "any-session", claude_base.path());

    assert!(
        result.is_none(),
        "should return None when project dir exists but has no jsonl files"
    );
}

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

// --- wait_for_session_log_or_exit ---

#[tokio::test]
#[serial_test::serial]
async fn wait_for_session_log_found_immediately() {
    let workspace_dir = TempDir::new().unwrap();
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let session_id = "test-session-found";

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

    let _ =
        wait_for_session_log_or_exit(workspace_dir.path(), "no-session", "trust-tmux", &sessions)
            .await;

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

// --- watch_agent ---

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
        owner: OwnerId::Job(JobId::default()),
    };

    let handle = tokio::spawn(watch_agent(config, sessions, event_tx, shutdown_rx, None));

    let event = tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await;
    assert!(
        matches!(event, Ok(Some(Event::AgentGone { .. }))),
        "expected AgentGone when session dies before log, got {:?}",
        event
    );

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
        owner: OwnerId::Job(JobId::default()),
    };

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(watch_agent(config, sessions, event_tx, shutdown_rx, None));

    tokio::time::sleep(Duration::from_millis(100)).await;

    sessions_clone.set_exited("alive-session", 1);

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
        owner: OwnerId::Job(JobId::default()),
    };

    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(watch_agent(config, sessions, event_tx, shutdown_rx, None));

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        event_rx.try_recv().is_err(),
        "no event for initial Working state"
    );

    sessions_clone.set_exited("tmux-found", 0);

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

// --- start_watcher ---

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
        owner: OwnerId::Job(JobId::default()),
    };

    let shutdown_tx = start_watcher(config, sessions, event_tx, None);

    let event = tokio::time::timeout(Duration::from_millis(500), event_rx.recv()).await;
    assert!(
        matches!(event, Ok(Some(Event::AgentGone { .. }))),
        "expected AgentGone from start_watcher, got {:?}",
        event
    );

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
        owner: OwnerId::Job(JobId::default()),
    };

    let shutdown_tx = start_watcher(config, sessions, event_tx, None);

    tokio::time::sleep(Duration::from_millis(50)).await;

    shutdown_tx.send(()).unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        event_rx.try_recv().is_err(),
        "no events after clean shutdown"
    );

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}

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
        owner: OwnerId::Job(JobId::default()),
    };

    let shutdown_tx = start_watcher(config, sessions, event_tx, Some(log_entry_tx));

    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = shutdown_tx.send(());

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}
