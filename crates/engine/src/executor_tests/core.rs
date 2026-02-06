// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for emit, timer, session, notify, execute_all, and accessor effects.

use super::*;

#[tokio::test]
async fn executor_emit_event_effect() {
    let harness = setup().await;

    // Emit returns the event and applies state
    let result = harness
        .executor
        .execute(Effect::Emit {
            event: Event::JobCreated {
                id: JobId::new("pipe-1"),
                kind: "build".to_string(),
                name: "test".to_string(),
                runbook_hash: "testhash".to_string(),
                cwd: std::path::PathBuf::from("/test"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_epoch_ms: 1_000_000,
                namespace: String::new(),
                cron_name: None,
            },
        })
        .await
        .unwrap();

    // Verify it returns the typed event
    assert!(result.is_some());
    assert!(matches!(result, Some(Event::JobCreated { .. })));

    // Verify state was applied
    let state = harness.executor.state();
    let state = state.lock();
    assert!(state.jobs.contains_key("pipe-1"));
}

#[tokio::test]
async fn executor_timer_effect() {
    let harness = setup().await;

    harness
        .executor
        .execute(Effect::SetTimer {
            id: TimerId::new("test-timer"),
            duration: std::time::Duration::from_secs(60),
        })
        .await
        .unwrap();

    let scheduler = harness.executor.scheduler();
    let scheduler = scheduler.lock();
    assert!(scheduler.has_timers());
}

#[tokio::test]
async fn cancel_timer_effect() {
    let harness = setup().await;

    // First set a timer
    harness
        .executor
        .execute(Effect::SetTimer {
            id: TimerId::new("timer-to-cancel"),
            duration: std::time::Duration::from_secs(60),
        })
        .await
        .unwrap();

    // Verify timer exists
    {
        let scheduler = harness.executor.scheduler();
        let scheduler = scheduler.lock();
        assert!(scheduler.has_timers());
    }

    // Cancel the timer
    harness
        .executor
        .execute(Effect::CancelTimer {
            id: TimerId::new("timer-to-cancel"),
        })
        .await
        .unwrap();

    // Verify timer is gone
    let scheduler = harness.executor.scheduler();
    let scheduler = scheduler.lock();
    assert!(!scheduler.has_timers());
}

#[tokio::test]
async fn send_to_session_effect_fails_for_nonexistent_session() {
    let harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::SendToSession {
            session_id: SessionId::new("nonexistent"),
            input: "continue\n".to_string(),
        })
        .await;

    // Send should fail because session doesn't exist
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[tokio::test]
async fn kill_session_effect() {
    let harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::KillSession {
            session_id: SessionId::new("sess-1"),
        })
        .await;

    // Kill should succeed with fake adapter
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn notify_effect_delegates_to_adapter() {
    let harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::Notify {
            title: "Test Title".to_string(),
            message: "Test message".to_string(),
        })
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());

    let calls = harness.notifier.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].title, "Test Title");
    assert_eq!(calls[0].message, "Test message");
}

#[tokio::test]
async fn multiple_notify_effects_recorded() {
    let harness = setup().await;

    harness
        .executor
        .execute(Effect::Notify {
            title: "First".to_string(),
            message: "msg1".to_string(),
        })
        .await
        .unwrap();
    harness
        .executor
        .execute(Effect::Notify {
            title: "Second".to_string(),
            message: "msg2".to_string(),
        })
        .await
        .unwrap();

    let calls = harness.notifier.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].title, "First");
    assert_eq!(calls[1].title, "Second");
}

// === execute_all tests ===

#[tokio::test]
async fn execute_all_collects_emitted_events() {
    let harness = setup().await;

    let effects = vec![
        Effect::Emit {
            event: Event::JobCreated {
                id: JobId::new("j-1"),
                kind: "build".to_string(),
                name: "first".to_string(),
                runbook_hash: "hash1".to_string(),
                cwd: std::path::PathBuf::from("/test"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_epoch_ms: 1_000,
                namespace: String::new(),
                cron_name: None,
            },
        },
        Effect::Emit {
            event: Event::JobCreated {
                id: JobId::new("j-2"),
                kind: "build".to_string(),
                name: "second".to_string(),
                runbook_hash: "hash2".to_string(),
                cwd: std::path::PathBuf::from("/test"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_epoch_ms: 2_000,
                namespace: String::new(),
                cron_name: None,
            },
        },
    ];

    let events = harness.executor.execute_all(effects).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], Event::JobCreated { .. }));
    assert!(matches!(events[1], Event::JobCreated { .. }));
}

#[tokio::test]
async fn execute_all_mixed_effects() {
    let mut harness = setup().await;

    // Mix of emit (returns event) and shell (returns None)
    let effects = vec![
        Effect::Emit {
            event: Event::JobCreated {
                id: JobId::new("j-mix"),
                kind: "build".to_string(),
                name: "mixed".to_string(),
                runbook_hash: "hash".to_string(),
                cwd: std::path::PathBuf::from("/test"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_epoch_ms: 1_000,
                namespace: String::new(),
                cron_name: None,
            },
        },
        Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("j-mix"))),
            step: "init".to_string(),
            command: "echo mixed".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        },
        Effect::Notify {
            title: "Done".to_string(),
            message: "mixed test".to_string(),
        },
    ];

    let inline_events = harness.executor.execute_all(effects).await.unwrap();
    // Only Emit produces an inline event; Shell and Notify do not
    assert_eq!(inline_events.len(), 1);
    assert!(matches!(inline_events[0], Event::JobCreated { .. }));

    // Shell event arrives via channel
    let shell_event = harness.event_rx.recv().await.unwrap();
    assert!(matches!(shell_event, Event::ShellExited { .. }));
}

// === Accessor method tests ===

#[tokio::test]
async fn check_session_alive_returns_false_for_nonexistent() {
    let harness = setup().await;

    let alive = harness
        .executor
        .check_session_alive("no-such-session")
        .await;
    assert!(!alive);
}

#[tokio::test]
async fn check_session_alive_returns_true_for_existing() {
    let harness = setup().await;

    // Add a session to the fake adapter
    harness.sessions.add_session("sess-alive", true);

    let alive = harness.executor.check_session_alive("sess-alive").await;
    assert!(alive);
}

#[tokio::test]
async fn check_process_running_returns_false_by_default() {
    let harness = setup().await;

    let running = harness
        .executor
        .check_process_running("sess-1", "claude")
        .await;
    assert!(!running);
}

#[tokio::test]
async fn get_agent_state_returns_state() {
    let harness = setup().await;

    // Spawn an agent first so it has state
    harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-state"),
            agent_name: "builder".to_string(),
            owner: OwnerId::Job(JobId::new("job-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: None,
            session_config: HashMap::new(),
        })
        .await
        .unwrap();

    let state = harness
        .executor
        .get_agent_state(&AgentId::new("agent-state"))
        .await;
    assert!(state.is_ok());
}

#[tokio::test]
async fn get_session_log_size_returns_none_for_unknown() {
    let harness = setup().await;

    let size = harness
        .executor
        .get_session_log_size(&AgentId::new("no-such-agent"))
        .await;
    assert!(size.is_none());
}

#[tokio::test]
async fn get_session_log_size_returns_value_when_set() {
    let harness = setup().await;

    let agent_id = AgentId::new("agent-log");

    // Spawn agent then set log size
    harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: agent_id.clone(),
            agent_name: "builder".to_string(),
            owner: OwnerId::Job(JobId::new("job-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: None,
            session_config: HashMap::new(),
        })
        .await
        .unwrap();

    harness.agents.set_session_log_size(&agent_id, Some(42));

    let size = harness.executor.get_session_log_size(&agent_id).await;
    assert_eq!(size, Some(42));
}

#[tokio::test]
async fn reconnect_agent_delegates_to_adapter() {
    let harness = setup().await;

    let config = AgentReconnectConfig {
        agent_id: AgentId::new("agent-recon"),
        session_id: "sess-recon".to_string(),
        workspace_path: std::path::PathBuf::from("/tmp/ws"),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::default()),
    };

    let result = harness.executor.reconnect_agent(config).await;
    assert!(result.is_ok());

    // Verify adapter was called
    let calls = harness.agents.calls();
    assert!(!calls.is_empty());
}

#[tokio::test]
async fn clock_accessor_returns_clock() {
    let harness = setup().await;

    // Just verify we can access the clock without panic
    let _now = oj_core::Clock::now(harness.executor.clock());
}
