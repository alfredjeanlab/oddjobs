// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::RuntimeDeps;
use oj_adapters::{FakeAgentAdapter, FakeSessionAdapter};
use oj_core::{FakeClock, PipelineId, SessionId, TimerId};
use std::collections::HashMap;
use tokio::sync::mpsc;

type TestExecutor = Executor<FakeSessionAdapter, FakeAgentAdapter, FakeClock>;

struct TestHarness {
    executor: TestExecutor,
    event_rx: mpsc::Receiver<Event>,
}

async fn setup() -> TestHarness {
    let (event_tx, event_rx) = mpsc::channel(100);

    let executor = Executor::new(
        RuntimeDeps {
            sessions: FakeSessionAdapter::new(),
            agents: FakeAgentAdapter::new(),
            state: Arc::new(Mutex::new(MaterializedState::default())),
        },
        Arc::new(Mutex::new(Scheduler::new())),
        FakeClock::new(),
        event_tx,
    );

    TestHarness { executor, event_rx }
}

#[tokio::test]
async fn executor_emit_event_effect() {
    let harness = setup().await;

    // Emit returns the event and applies state
    let result = harness
        .executor
        .execute(Effect::Emit {
            event: Event::PipelineCreated {
                id: PipelineId::new("pipe-1"),
                kind: "build".to_string(),
                name: "test".to_string(),
                runbook_hash: "testhash".to_string(),
                cwd: std::path::PathBuf::from("/test"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_epoch_ms: 1_000_000,
                namespace: String::new(),
            },
        })
        .await
        .unwrap();

    // Verify it returns the typed event
    assert!(result.is_some());
    assert!(matches!(result, Some(Event::PipelineCreated { .. })));

    // Verify state was applied
    let state = harness.executor.state();
    let state = state.lock();
    assert!(state.pipelines.contains_key("pipe-1"));
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
async fn shell_effect_runs_command() {
    let mut harness = setup().await;

    // execute() returns None immediately (spawned)
    let event = harness
        .executor
        .execute(Effect::Shell {
            pipeline_id: PipelineId::new("test"),
            step: "init".to_string(),
            command: "echo hello".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "shell effects return None (async)");

    // ShellExited arrives via event_tx
    let completed = harness.event_rx.recv().await.unwrap();
    assert!(matches!(completed, Event::ShellExited { exit_code: 0, .. }));
}

#[tokio::test]
async fn shell_failure_returns_nonzero() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::Shell {
            pipeline_id: PipelineId::new("test"),
            step: "init".to_string(),
            command: "exit 1".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "shell effects return None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    assert!(matches!(completed, Event::ShellExited { exit_code: 1, .. }));
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
async fn execute_all_shell_effects_are_async() {
    let mut harness = setup().await;

    let effects = vec![
        Effect::Shell {
            pipeline_id: PipelineId::new("pipe-1"),
            step: "init".to_string(),
            command: "echo first".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        },
        Effect::Shell {
            pipeline_id: PipelineId::new("pipe-1"),
            step: "build".to_string(),
            command: "echo second".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        },
    ];

    let inline_events = harness.executor.execute_all(effects).await.unwrap();
    assert!(
        inline_events.is_empty(),
        "shell effects produce no inline events"
    );

    // Both completions arrive via channel
    let e1 = harness.event_rx.recv().await.unwrap();
    let e2 = harness.event_rx.recv().await.unwrap();
    assert!(matches!(e1, Event::ShellExited { .. }));
    assert!(matches!(e2, Event::ShellExited { .. }));
}

#[tokio::test]
async fn notify_effect() {
    let harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::Notify {
            title: "Test Title".to_string(),
            message: "Test message".to_string(),
        })
        .await;

    // Notify should succeed (may fail silently if notifier not installed)
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn shell_intermediate_failure_propagates() {
    let mut harness = setup().await;

    // Multi-line command where an intermediate line fails.
    // With set -e, the first `false` should cause a nonzero exit.
    let event = harness
        .executor
        .execute(Effect::Shell {
            pipeline_id: PipelineId::new("test"),
            step: "init".to_string(),
            command: "false\ntrue".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "shell effects return None (async)");

    // The intermediate `false` must cause a nonzero exit code
    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited { exit_code, .. } => {
            assert_ne!(exit_code, 0, "intermediate failure should propagate");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_pipefail_propagates() {
    let mut harness = setup().await;

    // Pipeline where the first command fails but the second succeeds.
    // Without pipefail, `exit 1 | cat` would return 0.
    let event = harness
        .executor
        .execute(Effect::Shell {
            pipeline_id: PipelineId::new("test"),
            step: "init".to_string(),
            command: "exit 1 | cat".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "shell effects return None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited { exit_code, .. } => {
            assert_ne!(exit_code, 0, "pipe failure should propagate with pipefail");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}
