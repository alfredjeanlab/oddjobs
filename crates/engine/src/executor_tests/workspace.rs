// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for workspace effects (create, delete).

use super::*;

#[tokio::test]
async fn create_folder_workspace() {
    let mut harness = setup().await;
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

    // Should return WorkspaceCreated (deferred: background task does filesystem work)
    assert!(
        matches!(result, Some(Event::WorkspaceCreated { .. })),
        "expected WorkspaceCreated, got: {:?}",
        result
    );

    // WorkspaceReady arrives via event channel from background task
    let ready = tokio::time::timeout(std::time::Duration::from_secs(5), harness.event_rx.recv())
        .await
        .expect("timed out waiting for WorkspaceReady")
        .expect("channel closed");
    assert!(
        matches!(ready, Event::WorkspaceReady { .. }),
        "expected WorkspaceReady, got: {:?}",
        ready
    );

    // Directory should exist
    assert!(tmp.exists(), "workspace directory should be created");

    // State should have the workspace
    let state = harness.executor.state();
    let state = state.lock();
    assert!(state.workspaces.contains_key("ws-folder-1"));

    // Cleanup
    drop(state);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn create_folder_workspace_none_type() {
    let mut harness = setup().await;
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

    assert!(
        matches!(result, Some(Event::WorkspaceCreated { .. })),
        "expected WorkspaceCreated, got: {:?}",
        result
    );

    // WorkspaceReady arrives via event channel
    let ready = tokio::time::timeout(std::time::Duration::from_secs(5), harness.event_rx.recv())
        .await
        .expect("timed out waiting for WorkspaceReady")
        .expect("channel closed");
    assert!(matches!(ready, Event::WorkspaceReady { .. }));

    assert!(tmp.exists());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn create_workspace_failure_sends_failed_event() {
    let mut harness = setup().await;

    // Use a worktree workspace with an invalid repo_root to trigger failure
    let tmp = std::env::temp_dir().join("oj_test_create_ws_fail");
    let _ = std::fs::remove_dir_all(&tmp);

    let result = harness
        .executor
        .execute(Effect::CreateWorkspace {
            workspace_id: WorkspaceId::new("ws-fail-1"),
            path: tmp.join("workspace"),
            owner: Some(oj_core::OwnerId::Job(oj_core::JobId::new("job-fail"))),
            workspace_type: Some("worktree".to_string()),
            repo_root: Some(tmp.join("nonexistent-repo")),
            branch: Some("test-branch".to_string()),
            start_point: Some("HEAD".to_string()),
        })
        .await
        .unwrap();

    // Should return WorkspaceCreated immediately
    assert!(
        matches!(result, Some(Event::WorkspaceCreated { .. })),
        "expected WorkspaceCreated, got: {:?}",
        result
    );

    // WorkspaceFailed arrives via event channel from background task
    let failed = tokio::time::timeout(std::time::Duration::from_secs(5), harness.event_rx.recv())
        .await
        .expect("timed out waiting for WorkspaceFailed")
        .expect("channel closed");
    assert!(
        matches!(failed, Event::WorkspaceFailed { .. }),
        "expected WorkspaceFailed, got: {:?}",
        failed
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn delete_workspace_removes_plain_directory() {
    let mut harness = setup().await;
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

    // Deferred: returns Ok(None) immediately
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());

    // WorkspaceDeleted arrives via the event channel
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), harness.event_rx.recv())
        .await
        .expect("timed out waiting for WorkspaceDeleted")
        .expect("channel closed");
    assert!(
        matches!(event, Event::WorkspaceDeleted { .. }),
        "expected WorkspaceDeleted, got: {:?}",
        event
    );
    assert!(!tmp.exists(), "workspace directory should be removed");
}

#[tokio::test]
async fn delete_workspace_removes_git_worktree() {
    let mut harness = setup().await;

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

    // Deferred: returns Ok(None) immediately
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());

    // WorkspaceDeleted arrives via the event channel
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), harness.event_rx.recv())
        .await
        .expect("timed out waiting for WorkspaceDeleted")
        .expect("channel closed");
    assert!(
        matches!(event, Event::WorkspaceDeleted { .. }),
        "expected WorkspaceDeleted, got: {:?}",
        event
    );
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
    let mut harness = setup().await;

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

    // Deferred: returns Ok(None) immediately
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());

    // WorkspaceDeleted arrives via the event channel
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), harness.event_rx.recv())
        .await
        .expect("timed out waiting for WorkspaceDeleted")
        .expect("channel closed");
    assert!(
        matches!(event, Event::WorkspaceDeleted { .. }),
        "expected WorkspaceDeleted, got: {:?}",
        event
    );
}
