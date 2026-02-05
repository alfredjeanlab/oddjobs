// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::RuntimeDeps;
use oj_adapters::{FakeAgentAdapter, FakeNotifyAdapter, FakeSessionAdapter};
use oj_core::{FakeClock, JobId, OwnerId, SessionId, TimerId, WorkspaceId};
use std::collections::HashMap;
use tokio::sync::mpsc;

type TestExecutor = Executor<FakeSessionAdapter, FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

struct TestHarness {
    executor: TestExecutor,
    event_rx: mpsc::Receiver<Event>,
    notifier: FakeNotifyAdapter,
}

async fn setup() -> TestHarness {
    let (event_tx, event_rx) = mpsc::channel(100);
    let notifier = FakeNotifyAdapter::new();

    let executor = Executor::new(
        RuntimeDeps {
            sessions: FakeSessionAdapter::new(),
            agents: FakeAgentAdapter::new(),
            notifier: notifier.clone(),
            state: Arc::new(Mutex::new(MaterializedState::default())),
        },
        Arc::new(Mutex::new(Scheduler::new())),
        FakeClock::new(),
        event_tx,
    );

    TestHarness {
        executor,
        event_rx,
        notifier,
    }
}

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
async fn shell_effect_runs_command() {
    let mut harness = setup().await;

    // execute() returns None immediately (spawned)
    let event = harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
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
            owner: Some(OwnerId::Job(JobId::new("test"))),
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
            owner: Some(OwnerId::Job(JobId::new("pipe-1"))),
            step: "init".to_string(),
            command: "echo first".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        },
        Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("pipe-1"))),
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

#[tokio::test]
async fn shell_intermediate_failure_propagates() {
    let mut harness = setup().await;

    // Multi-line command where an intermediate line fails.
    // With set -e, the first `false` should cause a nonzero exit.
    let event = harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
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

    // Job where the first command fails but the second succeeds.
    // Without pipefail, `exit 1 | cat` would return 0.
    let event = harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
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

#[tokio::test]
async fn delete_workspace_removes_plain_directory() {
    let harness = setup().await;
    let tmp = std::env::temp_dir().join("oj_test_delete_ws_plain");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // Insert workspace record into state
    {
        let state_arc = harness.executor.state();
        let mut state = state_arc.lock();
        state.workspaces.insert(
            "ws-plain".to_string(),
            oj_storage::Workspace {
                id: "ws-plain".to_string(),
                path: tmp.clone(),
                branch: None,
                owner: None,
                status: oj_core::WorkspaceStatus::Ready,
                workspace_type: oj_storage::WorkspaceType::Folder,
                created_at_ms: 0,
            },
        );
    }

    let result = harness
        .executor
        .execute(Effect::DeleteWorkspace {
            workspace_id: WorkspaceId::new("ws-plain"),
        })
        .await;

    assert!(result.is_ok());
    assert!(matches!(
        result.unwrap(),
        Some(Event::WorkspaceDeleted { .. })
    ));
    assert!(!tmp.exists(), "workspace directory should be removed");
}

#[tokio::test]
async fn delete_workspace_removes_git_worktree() {
    let harness = setup().await;

    // Create a temporary git repo and a worktree from it
    let base = std::env::temp_dir().join("oj_test_delete_ws_wt");
    let _ = std::fs::remove_dir_all(&base);
    let repo_dir = base.join("repo");
    let wt_dir = base.join("worktree");
    std::fs::create_dir_all(&repo_dir).unwrap();

    // Initialize a git repo with an initial commit.
    // Clear GIT_DIR/GIT_WORK_TREE so this works inside worktrees.
    let init = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(init.status.success(), "git init failed");

    let commit = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(&repo_dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(commit.status.success(), "git commit failed");

    // Create a worktree
    let add_wt = std::process::Command::new("git")
        .args(["worktree", "add", wt_dir.to_str().unwrap(), "HEAD"])
        .current_dir(&repo_dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(add_wt.status.success(), "git worktree add failed");

    // Verify worktree .git is a file (not a directory)
    let dot_git = wt_dir.join(".git");
    assert!(dot_git.is_file(), ".git should be a file in a worktree");

    // Insert workspace record into state
    {
        let state_arc = harness.executor.state();
        let mut state = state_arc.lock();
        state.workspaces.insert(
            "ws-wt".to_string(),
            oj_storage::Workspace {
                id: "ws-wt".to_string(),
                path: wt_dir.clone(),
                branch: None,
                owner: None,
                status: oj_core::WorkspaceStatus::Ready,
                workspace_type: oj_storage::WorkspaceType::Folder,
                created_at_ms: 0,
            },
        );
    }

    let result = harness
        .executor
        .execute(Effect::DeleteWorkspace {
            workspace_id: WorkspaceId::new("ws-wt"),
        })
        .await;

    assert!(result.is_ok());
    assert!(matches!(
        result.unwrap(),
        Some(Event::WorkspaceDeleted { .. })
    ));
    assert!(!wt_dir.exists(), "worktree directory should be removed");

    // Verify git no longer lists the worktree
    let list = std::process::Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&repo_dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    let output = String::from_utf8_lossy(&list.stdout);
    // Should only have the main repo worktree, not the deleted one
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "should only have main worktree listed, got: {output}"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn take_queue_item_effect_runs_async() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::TakeQueueItem {
            worker_name: "test-worker".to_string(),
            take_command: "echo taken".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            item_id: "item-1".to_string(),
            item: serde_json::json!({"id": "item-1", "title": "test"}),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "TakeQueueItem should return None (async)");

    // WorkerTakeComplete arrives via event_tx
    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerTakeComplete {
            worker_name,
            item_id,
            exit_code,
            ..
        } => {
            assert_eq!(worker_name, "test-worker");
            assert_eq!(item_id, "item-1");
            assert_eq!(exit_code, 0);
        }
        other => panic!("expected WorkerTakeComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn take_queue_item_failure_returns_nonzero() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::TakeQueueItem {
            worker_name: "test-worker".to_string(),
            take_command: "exit 1".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            item_id: "item-2".to_string(),
            item: serde_json::json!({"id": "item-2"}),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "TakeQueueItem should return None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerTakeComplete {
            exit_code, item_id, ..
        } => {
            assert_eq!(item_id, "item-2");
            assert_ne!(exit_code, 0, "failed take should have nonzero exit code");
        }
        other => panic!("expected WorkerTakeComplete, got {:?}", other),
    }
}
