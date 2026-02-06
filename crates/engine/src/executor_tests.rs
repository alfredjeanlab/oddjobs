// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::RuntimeDeps;
use oj_adapters::{
    AgentAdapterError, AgentReconnectConfig, FakeAgentAdapter, FakeNotifyAdapter,
    FakeSessionAdapter,
};
use oj_core::{AgentId, AgentRunId, FakeClock, JobId, OwnerId, SessionId, TimerId, WorkspaceId};
use std::collections::HashMap;
use tokio::sync::mpsc;

type TestExecutor = Executor<FakeSessionAdapter, FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

struct TestHarness {
    executor: TestExecutor,
    event_rx: mpsc::Receiver<Event>,
    sessions: FakeSessionAdapter,
    agents: FakeAgentAdapter,
    notifier: FakeNotifyAdapter,
}

async fn setup() -> TestHarness {
    let (event_tx, event_rx) = mpsc::channel(100);
    let sessions = FakeSessionAdapter::new();
    let agents = FakeAgentAdapter::new();
    let notifier = FakeNotifyAdapter::new();

    let executor = Executor::new(
        RuntimeDeps {
            sessions: sessions.clone(),
            agents: agents.clone(),
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
        sessions,
        agents,
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

// === SpawnAgent tests ===

#[tokio::test]
async fn spawn_agent_returns_session_created() {
    let harness = setup().await;

    let mut input = HashMap::new();
    input.insert("prompt".to_string(), "do the thing".to_string());
    input.insert("name".to_string(), "test-job".to_string());

    let result = harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-1"),
            agent_name: "builder".to_string(),
            owner: OwnerId::Job(JobId::new("job-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input,
            command: "claude".to_string(),
            env: vec![("FOO".to_string(), "bar".to_string())],
            cwd: None,
            session_config: HashMap::new(),
        })
        .await
        .unwrap();

    // Should return SessionCreated event
    assert!(matches!(result, Some(Event::SessionCreated { .. })));

    // Verify state was updated with the session
    let state = harness.executor.state();
    let state = state.lock();
    assert!(
        !state.sessions.is_empty(),
        "session should be tracked in state"
    );

    // Verify agent adapter was called
    let calls = harness.agents.calls();
    assert_eq!(calls.len(), 1);
}

#[tokio::test]
async fn spawn_agent_with_agent_run_owner() {
    let harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-2"),
            agent_name: "runner".to_string(),
            owner: OwnerId::AgentRun(AgentRunId::new("ar-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws2"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: Some(std::path::PathBuf::from("/tmp")),
            session_config: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(matches!(result, Some(Event::SessionCreated { .. })));

    // Verify the event has the correct owner
    if let Some(Event::SessionCreated { owner, .. }) = result {
        assert!(matches!(owner, OwnerId::AgentRun(_)));
    }
}

#[tokio::test]
async fn spawn_agent_error_propagates() {
    let harness = setup().await;

    // Inject a spawn error
    harness
        .agents
        .set_spawn_error(AgentAdapterError::SpawnFailed("test failure".to_string()));

    let result = harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-err"),
            agent_name: "builder".to_string(),
            owner: OwnerId::Job(JobId::new("job-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: None,
            session_config: HashMap::new(),
        })
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("test failure"));
}

// === SendToAgent / KillAgent tests ===

#[tokio::test]
async fn send_to_agent_delegates_to_adapter() {
    let harness = setup().await;

    // First spawn an agent so it exists
    harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-send"),
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

    let result = harness
        .executor
        .execute(Effect::SendToAgent {
            agent_id: AgentId::new("agent-send"),
            input: "continue working".to_string(),
        })
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn send_to_agent_error_propagates() {
    let harness = setup().await;

    harness
        .agents
        .set_send_error(AgentAdapterError::NotFound("agent-missing".to_string()));

    let result = harness
        .executor
        .execute(Effect::SendToAgent {
            agent_id: AgentId::new("agent-missing"),
            input: "hello".to_string(),
        })
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("agent-missing"));
}

#[tokio::test]
async fn kill_agent_delegates_to_adapter() {
    let harness = setup().await;

    // Spawn an agent first so it can be killed
    harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-kill"),
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

    let result = harness
        .executor
        .execute(Effect::KillAgent {
            agent_id: AgentId::new("agent-kill"),
        })
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn kill_agent_error_propagates() {
    let harness = setup().await;

    harness
        .agents
        .set_kill_error(AgentAdapterError::NotFound("agent-gone".to_string()));

    let result = harness
        .executor
        .execute(Effect::KillAgent {
            agent_id: AgentId::new("agent-gone"),
        })
        .await;

    assert!(result.is_err());
}

// === PollQueue tests ===

#[tokio::test]
async fn poll_queue_with_valid_json() {
    let mut harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::PollQueue {
            worker_name: "poller".to_string(),
            list_command: r#"echo '[{"id":"1"},{"id":"2"}]'"#.to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        })
        .await
        .unwrap();

    assert!(result.is_none(), "PollQueue returns None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerPollComplete {
            worker_name, items, ..
        } => {
            assert_eq!(worker_name, "poller");
            assert_eq!(items.len(), 2);
            assert_eq!(items[0]["id"], "1");
            assert_eq!(items[1]["id"], "2");
        }
        other => panic!("expected WorkerPollComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn poll_queue_with_empty_output() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::PollQueue {
            worker_name: "poller".to_string(),
            list_command: "echo '[]'".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerPollComplete { items, .. } => {
            assert!(items.is_empty());
        }
        other => panic!("expected WorkerPollComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn poll_queue_with_invalid_json_returns_empty() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::PollQueue {
            worker_name: "poller".to_string(),
            list_command: "echo 'not json'".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerPollComplete { items, .. } => {
            assert!(
                items.is_empty(),
                "invalid JSON should result in empty items"
            );
        }
        other => panic!("expected WorkerPollComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn poll_queue_command_failure_returns_empty() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::PollQueue {
            worker_name: "poller".to_string(),
            list_command: "exit 1".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerPollComplete { items, .. } => {
            assert!(
                items.is_empty(),
                "failed command should result in empty items"
            );
        }
        other => panic!("expected WorkerPollComplete, got {:?}", other),
    }
}

// === CreateWorkspace tests ===

#[tokio::test]
async fn create_folder_workspace() {
    let harness = setup().await;
    let tmp = std::env::temp_dir().join("oj_test_create_ws_folder");
    let _ = std::fs::remove_dir_all(&tmp);

    let result = harness
        .executor
        .execute(Effect::CreateWorkspace {
            workspace_id: WorkspaceId::new("ws-folder-1"),
            path: tmp.clone(),
            owner: Some(oj_core::OwnerId::Job(oj_core::JobId::new("job-1"))),
            workspace_type: Some("folder".to_string()),
            repo_root: None,
            branch: None,
            start_point: None,
        })
        .await
        .unwrap();

    // Should return WorkspaceReady
    assert!(matches!(result, Some(Event::WorkspaceReady { .. })));

    // Directory should exist
    assert!(tmp.exists(), "workspace directory should be created");

    // State should have the workspace
    let state = harness.executor.state();
    let state = state.lock();
    assert!(state.workspaces.contains_key("ws-folder-1"));

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn create_folder_workspace_none_type() {
    let harness = setup().await;
    let tmp = std::env::temp_dir().join("oj_test_create_ws_none_type");
    let _ = std::fs::remove_dir_all(&tmp);

    // workspace_type=None should fall through to folder creation
    let result = harness
        .executor
        .execute(Effect::CreateWorkspace {
            workspace_id: WorkspaceId::new("ws-none-type"),
            path: tmp.clone(),
            owner: None,
            workspace_type: None,
            repo_root: None,
            branch: None,
            start_point: None,
        })
        .await
        .unwrap();

    assert!(matches!(result, Some(Event::WorkspaceReady { .. })));
    assert!(tmp.exists());

    let _ = std::fs::remove_dir_all(&tmp);
}

// === DeleteWorkspace edge cases ===

#[tokio::test]
async fn delete_workspace_not_found_returns_error() {
    let harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::DeleteWorkspace {
            workspace_id: WorkspaceId::new("nonexistent-ws"),
        })
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ExecuteError::WorkspaceNotFound(id) => {
            assert_eq!(id, "nonexistent-ws");
        }
        other => panic!("expected WorkspaceNotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn delete_workspace_already_removed_directory() {
    let harness = setup().await;

    // Insert a workspace record pointing to a directory that doesn't exist
    let nonexistent_path = std::env::temp_dir().join("oj_test_already_gone");
    let _ = std::fs::remove_dir_all(&nonexistent_path);

    {
        let state_arc = harness.executor.state();
        let mut state = state_arc.lock();
        state.workspaces.insert(
            "ws-gone".to_string(),
            oj_storage::Workspace {
                id: "ws-gone".to_string(),
                path: nonexistent_path,
                branch: None,
                owner: None,
                status: oj_core::WorkspaceStatus::Ready,
                workspace_type: oj_storage::WorkspaceType::Folder,
                created_at_ms: 0,
            },
        );
    }

    // Should succeed even if the directory doesn't exist
    let result = harness
        .executor
        .execute(Effect::DeleteWorkspace {
            workspace_id: WorkspaceId::new("ws-gone"),
        })
        .await;

    assert!(result.is_ok());
    assert!(matches!(
        result.unwrap(),
        Some(Event::WorkspaceDeleted { .. })
    ));
}

// === Shell edge cases ===

#[tokio::test]
async fn shell_captures_stdout_and_stderr() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "echo stdout_output && echo stderr_output >&2".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code,
            stdout,
            stderr,
            ..
        } => {
            assert_eq!(exit_code, 0);
            assert_eq!(stdout.unwrap().trim(), "stdout_output");
            assert_eq!(stderr.unwrap().trim(), "stderr_output");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_with_env_vars() {
    let mut harness = setup().await;

    let mut env = HashMap::new();
    env.insert("MY_TEST_VAR".to_string(), "hello_from_env".to_string());

    harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "echo $MY_TEST_VAR".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env,
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code, stdout, ..
        } => {
            assert_eq!(exit_code, 0);
            assert_eq!(stdout.unwrap().trim(), "hello_from_env");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_with_none_owner() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::Shell {
            owner: None,
            step: "init".to_string(),
            command: "echo no_owner".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code, stdout, ..
        } => {
            assert_eq!(exit_code, 0);
            assert_eq!(stdout.unwrap().trim(), "no_owner");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_with_agent_run_owner() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::AgentRun(AgentRunId::new("ar-1"))),
            step: "run".to_string(),
            command: "echo agent_run_shell".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code, stdout, ..
        } => {
            assert_eq!(exit_code, 0);
            assert_eq!(stdout.unwrap().trim(), "agent_run_shell");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_no_stdout_when_empty() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "true".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code,
            stdout,
            stderr,
            ..
        } => {
            assert_eq!(exit_code, 0);
            assert!(stdout.is_none(), "empty stdout should be None");
            assert!(stderr.is_none(), "empty stderr should be None");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

// === TakeQueueItem edge cases ===

#[tokio::test]
async fn take_queue_item_with_stderr() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::TakeQueueItem {
            worker_name: "test-worker".to_string(),
            take_command: "echo stderr_msg >&2 && exit 1".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            item_id: "item-3".to_string(),
            item: serde_json::json!({"id": "item-3"}),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerTakeComplete {
            exit_code, stderr, ..
        } => {
            assert_ne!(exit_code, 0);
            assert!(stderr.is_some());
            assert!(stderr.unwrap().contains("stderr_msg"));
        }
        other => panic!("expected WorkerTakeComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn take_queue_item_preserves_item_data() {
    let mut harness = setup().await;

    let item_data = serde_json::json!({
        "id": "item-4",
        "title": "Important task",
        "priority": 1
    });

    harness
        .executor
        .execute(Effect::TakeQueueItem {
            worker_name: "test-worker".to_string(),
            take_command: "true".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            item_id: "item-4".to_string(),
            item: item_data.clone(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerTakeComplete { item, item_id, .. } => {
            assert_eq!(item_id, "item-4");
            assert_eq!(item, item_data);
        }
        other => panic!("expected WorkerTakeComplete, got {:?}", other),
    }
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
