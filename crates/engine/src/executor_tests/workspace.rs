// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for workspace effects (create, delete).

use super::*;

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
