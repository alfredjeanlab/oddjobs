// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

// --- find_session_log Tests ---

#[test]
fn find_session_log_requires_correct_workspace_path() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();

    let session_id = "test-session";
    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join(format!("{session_id}.jsonl")),
        r#"{"type":"user","message":{"content":"hello"}}"#,
    )
    .unwrap();

    assert!(
        find_session_log_in(workspace_dir.path(), session_id, claude_base.path()).is_some(),
        "should find session log when given the workspace path"
    );
    assert!(
        find_session_log_in(project_dir.path(), session_id, claude_base.path()).is_none(),
        "should not find session log when given project_root (different hash)"
    );
}

#[test]
fn find_session_log_in_uses_fallback_for_missing_session() {
    let (claude_base, workspace_dir, log_dir) = setup_claude_project("existing");

    let other_session_path = log_dir.join("other-session.jsonl");
    std::fs::write(&other_session_path, r#"{"type":"user"}"#).unwrap();

    let result = find_session_log_in(
        workspace_dir.path(),
        "nonexistent-session",
        claude_base.path(),
    );
    assert_eq!(result, Some(other_session_path));
}

#[test]
fn find_session_log_in_returns_none_for_missing_project() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    assert!(find_session_log_in(workspace_dir.path(), "any-session", claude_base.path()).is_none());
}

#[test]
fn find_session_log_in_picks_most_recent_fallback() {
    let (claude_base, workspace_dir, log_dir) = setup_claude_project("existing");

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
    assert_eq!(
        result,
        Some(newer),
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

    assert!(
        find_session_log_in(workspace_dir.path(), "any-session", claude_base.path()).is_none(),
        "should return None when project dir exists but has no jsonl files"
    );
}

// --- find_session_log with CLAUDE_CONFIG_DIR Tests ---

#[test]
#[serial_test::serial]
fn find_session_log_uses_claude_config_dir_env() {
    let (claude_base, workspace_dir, log_dir) = setup_claude_project("env-var-session");
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());

    let session_file = log_dir.join("env-var-session.jsonl");
    let result = find_session_log(workspace_dir.path(), "env-var-session");
    assert_eq!(result, Some(session_file));

    std::env::remove_var("CLAUDE_CONFIG_DIR");
}

#[test]
#[serial_test::serial]
fn find_session_log_returns_none_when_no_log_exists() {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());

    assert!(find_session_log(workspace_dir.path(), "nonexistent").is_none());

    std::env::remove_var("CLAUDE_CONFIG_DIR");
}

// --- check_liveness Tests ---

#[tokio::test]
async fn check_liveness_returns_none_when_alive() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_process_running("test-session", true);

    let agent_id = AgentId::new("test-agent");
    let result = check_liveness(&sessions, "test-session", "claude", &agent_id).await;
    assert!(result.is_none(), "should return None when session and process are alive");
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

// --- check_and_accept_trust_prompt Tests ---

#[tokio::test]
async fn check_trust_prompt_detected_and_accepted() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_output(
        "test-session",
        vec!["Do you trust the files in this folder?".to_string()],
    );

    assert!(check_and_accept_trust_prompt(&sessions, "test-session").await);

    let calls = sessions.calls();
    assert!(
        calls.iter().any(|c| matches!(
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
    assert!(check_and_accept_trust_prompt(&sessions, "test-session").await);
}

#[tokio::test]
async fn check_trust_prompt_not_present() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);
    sessions.set_output(
        "test-session",
        vec!["Welcome to Claude".to_string(), "How can I help?".to_string()],
    );
    assert!(!check_and_accept_trust_prompt(&sessions, "test-session").await);
}

#[tokio::test]
async fn check_trust_prompt_capture_error_returns_false() {
    let sessions = FakeSessionAdapter::new();
    assert!(!check_and_accept_trust_prompt(&sessions, "nonexistent").await);
}

// --- wait_for_session_log_or_exit Tests ---

#[tokio::test]
#[serial_test::serial]
async fn wait_for_session_log_found_immediately() {
    let (claude_base, workspace_dir, _log_dir) = setup_claude_project("test-session-found");
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("tmux-session", true);

    let result = wait_for_session_log_or_exit(
        workspace_dir.path(),
        "test-session-found",
        "tmux-session",
        &sessions,
    )
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
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let workspace_dir = TempDir::new().unwrap();
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("dead-tmux", false);

    let result = wait_for_session_log_or_exit(
        workspace_dir.path(),
        "nonexistent-session",
        "dead-tmux",
        &sessions,
    )
    .await;

    assert!(matches!(result, SessionLogWait::SessionDied));

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
}

#[tokio::test]
#[serial_test::serial]
async fn wait_for_session_log_timeout() {
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let workspace_dir = TempDir::new().unwrap();
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("alive-tmux", true);

    let result = wait_for_session_log_or_exit(
        workspace_dir.path(),
        "never-created-session",
        "alive-tmux",
        &sessions,
    )
    .await;

    assert!(matches!(result, SessionLogWait::Timeout));

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
}

#[tokio::test]
#[serial_test::serial]
async fn wait_for_session_log_checks_trust_prompt_early() {
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");

    let workspace_dir = TempDir::new().unwrap();
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("trust-tmux", true);
    sessions.set_output(
        "trust-tmux",
        vec!["Do you trust the files in this folder?".to_string()],
    );

    let _ = wait_for_session_log_or_exit(
        workspace_dir.path(),
        "no-session",
        "trust-tmux",
        &sessions,
    )
    .await;

    let calls = sessions.calls();
    assert!(
        calls.iter().any(|c| matches!(c, SessionCall::Send { input, .. } if input == "y")),
        "should send 'y' for trust prompt during early iterations"
    );

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
}

// --- watch_agent full lifecycle tests ---

#[tokio::test]
#[serial_test::serial]
async fn watch_agent_emits_agent_gone_when_session_dies_before_log() {
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let workspace_dir = TempDir::new().unwrap();
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("dead-session", false);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let config = test_watcher_config("nonexistent-log", "dead-session", workspace_dir.path());
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
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let workspace_dir = TempDir::new().unwrap();
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("alive-session", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let config = test_watcher_config("never-created", "alive-session", workspace_dir.path());
    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(watch_agent(config, sessions, event_tx, shutdown_rx, None));

    // Wait for timeout (30 iterations * 1ms), then fallback polling starts
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
    let (claude_base, workspace_dir, _log_dir) = setup_claude_project("found-session");
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("tmux-found", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let config = test_watcher_config("found-session", "tmux-found", workspace_dir.path());
    let sessions_clone = sessions.clone();
    let handle = tokio::spawn(watch_agent(config, sessions, event_tx, shutdown_rx, None));

    // Let it enter watch_loop
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(event_rx.try_recv().is_err(), "no event for initial Working state");

    // Kill the session to trigger liveness check
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

// --- start_watcher Tests ---

#[tokio::test]
#[serial_test::serial]
async fn start_watcher_returns_shutdown_sender() {
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let workspace_dir = TempDir::new().unwrap();
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("start-watcher-tmux", false);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let config = test_watcher_config("start-watcher-session", "start-watcher-tmux", workspace_dir.path());
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
    let (claude_base, workspace_dir, _log_dir) = setup_claude_project("shutdown-session");
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1000");
    std::env::set_var("OJ_WATCHER_POLL_MS", "5000");

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("shutdown-tmux", true);

    let (event_tx, mut event_rx) = mpsc::channel(32);
    let config = test_watcher_config("shutdown-session", "shutdown-tmux", workspace_dir.path());
    let shutdown_tx = start_watcher(config, sessions, event_tx, None);

    // Let it start and enter watch_loop
    tokio::time::sleep(Duration::from_millis(50)).await;
    shutdown_tx.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(event_rx.try_recv().is_err(), "no events after clean shutdown");

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}

// --- start_watcher with log_entry_tx ---

#[tokio::test]
#[serial_test::serial]
async fn start_watcher_with_log_entry_tx() {
    let claude_base = TempDir::new().unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", claude_base.path());
    std::env::set_var("OJ_SESSION_POLL_MS", "1");
    std::env::set_var("OJ_WATCHER_POLL_MS", "10");

    let workspace_dir = TempDir::new().unwrap();
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("log-entry-tmux", false);

    let (event_tx, _event_rx) = mpsc::channel(32);
    let (log_entry_tx, _log_entry_rx) = mpsc::channel(32);

    let config = test_watcher_config("log-entry-session", "log-entry-tmux", workspace_dir.path());
    let shutdown_tx = start_watcher(config, sessions, event_tx, Some(log_entry_tx));

    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = shutdown_tx.send(());

    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::remove_var("OJ_SESSION_POLL_MS");
    std::env::remove_var("OJ_WATCHER_POLL_MS");
}
