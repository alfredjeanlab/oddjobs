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
                id: JobId::new("job-1"),
                kind: "build".to_string(),
                name: "test".to_string(),
                runbook_hash: "testhash".to_string(),
                cwd: std::path::PathBuf::from("/test"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_ms: 1_000_000,
                project: String::new(),
                cron: None,
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
    assert!(state.jobs.contains_key("job-1"));
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
        .execute(Effect::CancelTimer { id: TimerId::new("timer-to-cancel") })
        .await
        .unwrap();

    // Verify timer is gone
    let scheduler = harness.executor.scheduler();
    let scheduler = scheduler.lock();
    assert!(!scheduler.has_timers());
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
        .execute(Effect::Notify { title: "First".to_string(), message: "msg1".to_string() })
        .await
        .unwrap();
    harness
        .executor
        .execute(Effect::Notify { title: "Second".to_string(), message: "msg2".to_string() })
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
                created_at_ms: 1_000,
                project: String::new(),
                cron: None,
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
                created_at_ms: 2_000,
                project: String::new(),
                cron: None,
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
                created_at_ms: 1_000,
                project: String::new(),
                cron: None,
            },
        },
        Effect::Shell {
            owner: Some(JobId::new("j-mix").into()),
            step: "init".to_string(),
            command: "echo mixed".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
            container: None,
        },
        Effect::Notify { title: "Done".to_string(), message: "mixed test".to_string() },
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
async fn check_agent_alive_returns_false_for_unknown() {
    let harness = setup().await;

    let alive = harness.executor.agents.is_alive(&AgentId::new("no-such-agent")).await;
    assert!(!alive);
}

#[tokio::test]
async fn check_agent_alive_returns_true_after_spawn() {
    let mut harness = setup().await;

    // Spawn an agent — FakeAgentAdapter defaults alive=true
    harness.executor.execute(spawn_agent("agent-alive")).await.unwrap();

    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), harness.event_rx.recv()).await;

    let alive = harness.executor.agents.is_alive(&AgentId::new("agent-alive")).await;
    assert!(alive);
}

#[tokio::test]
async fn check_agent_alive_returns_false_when_marked_dead() {
    let mut harness = setup().await;

    let agent_id = AgentId::new("agent-dead");
    harness.executor.execute(spawn_agent("agent-dead")).await.unwrap();

    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), harness.event_rx.recv()).await;

    harness.agents.set_agent_alive(&agent_id, false);
    let alive = harness.executor.agents.is_alive(&agent_id).await;
    assert!(!alive);
}

#[tokio::test]
async fn get_agent_state_returns_state() {
    let mut harness = setup().await;

    // Spawn an agent first so it has state
    harness.executor.execute(spawn_agent("agent-state")).await.unwrap();

    // Wait for the background spawn task to complete
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), harness.event_rx.recv()).await;

    let state = harness.executor.get_agent_state(&AgentId::new("agent-state")).await;
    assert!(state.is_ok());
}

#[tokio::test]
async fn reconnect_agent_delegates_to_adapter() {
    let harness = setup().await;

    let config = AgentReconnectConfig {
        agent_id: AgentId::new("agent-recon"),
        workspace_path: std::path::PathBuf::from("/tmp/ws"),
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
