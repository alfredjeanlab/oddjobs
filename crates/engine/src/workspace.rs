// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace effect execution (create/delete worktrees and folders).

use crate::executor::ExecuteError;
use oj_core::Event;
use oj_storage::MaterializedState;

use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Execute a CreateWorkspace effect.
///
/// Immediately records the workspace in state and spawns a background task
/// for the filesystem work (worktree add or mkdir). Returns the
/// `WorkspaceCreated` event for WAL persistence.
// TODO(refactor): group workspace creation params into a context struct
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create(
    state: &Arc<Mutex<MaterializedState>>,
    event_tx: &mpsc::Sender<Event>,
    workspace_id: oj_core::WorkspaceId,
    path: PathBuf,
    owner: oj_core::OwnerId,
    workspace_type: Option<String>,
    repo_root: Option<PathBuf>,
    branch: Option<String>,
    start_point: Option<String>,
) -> Result<Option<Event>, ExecuteError> {
    let is_worktree = workspace_type.as_deref() == Some("worktree");

    // Phase 1: Create workspace record immediately so job.workspace_path is set
    let create_event = Event::WorkspaceCreated {
        id: workspace_id.clone(),
        path: path.clone(),
        branch: branch.clone(),
        owner: owner.clone(),
        workspace_type,
    };
    {
        let mut state = state.lock();
        state.apply_event(&create_event);
    }

    // Phase 2: Spawn background task for filesystem work
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        let result = if is_worktree {
            create_worktree(&path, repo_root, branch, start_point).await
        } else {
            create_folder(&path).await
        };

        let event = match result {
            Ok(()) => Event::WorkspaceReady { id: workspace_id },
            Err(reason) => Event::WorkspaceFailed { id: workspace_id, reason },
        };

        if let Err(e) = event_tx.send(event).await {
            tracing::error!("failed to send workspace event: {}", e);
        }
    });

    // Return WorkspaceCreated for WAL persistence (background task sends Ready/Failed)
    Ok(Some(create_event))
}

/// Execute a DeleteWorkspace effect.
///
/// Looks up the workspace synchronously and spawns a background task
/// for the filesystem work (worktree remove, directory deletion).
/// The background task emits `WorkspaceDeleted` via `event_tx` on
/// completion.
pub(crate) async fn delete(
    state: &Arc<Mutex<MaterializedState>>,
    event_tx: &mpsc::Sender<Event>,
    workspace_id: oj_core::WorkspaceId,
) -> Result<Option<Event>, ExecuteError> {
    // Look up workspace path and branch (synchronous, fast)
    let (workspace_path, workspace_branch) = {
        let state = state.lock();
        let ws = state
            .workspaces
            .get(workspace_id.as_str())
            .ok_or_else(|| ExecuteError::WorkspaceNotFound(workspace_id.to_string()))?;
        (ws.path.clone(), ws.branch.clone())
    };

    // Update status to Cleaning (transient, not persisted)
    {
        let mut state = state.lock();
        if let Some(workspace) = state.workspaces.get_mut(workspace_id.as_str()) {
            workspace.status = oj_core::WorkspaceStatus::Cleaning;
        }
    }

    // Spawn background task for filesystem work
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        delete_workspace_files(&workspace_path, &workspace_branch).await;

        let event = Event::WorkspaceDeleted { id: workspace_id.clone() };

        if let Err(e) = event_tx.send(event).await {
            tracing::error!("failed to send WorkspaceDeleted: {}", e);
        }
    });

    Ok(None)
}

/// Remove workspace files from disk (git worktree + directory).
///
/// All operations are best-effort — errors are logged but don't
/// prevent the `WorkspaceDeleted` event from being emitted.
async fn delete_workspace_files(
    workspace_path: &std::path::Path,
    workspace_branch: &Option<String>,
) {
    // If the workspace is a git worktree, unregister it first
    let dot_git = workspace_path.join(".git");
    if tokio::fs::symlink_metadata(&dot_git).await.map(|m| m.is_file()).unwrap_or(false) {
        // Best-effort: git worktree remove --force
        // Run from within the worktree so git can locate the parent repo.
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("worktree")
            .arg("remove")
            .arg("--force")
            .arg(workspace_path)
            .current_dir(workspace_path);
        let _ = oj_adapters::subprocess::run_with_timeout(
            cmd,
            oj_adapters::subprocess::GIT_WORKTREE_TIMEOUT,
            "git worktree remove",
        )
        .await;

        // Best-effort: clean up the branch
        if let Some(ref branch) = workspace_branch {
            // Find the repo root from the worktree's .git file
            if let Ok(contents) = tokio::fs::read_to_string(&dot_git).await {
                // .git file contains: gitdir: /path/to/repo/.git/worktrees/<name>
                if let Some(gitdir) = contents.trim().strip_prefix("gitdir: ") {
                    // Navigate up from .git/worktrees/<name> to .git, then parent
                    let gitdir_path = std::path::Path::new(gitdir);
                    if let Some(repo_root) =
                        gitdir_path.parent().and_then(|p| p.parent()).and_then(|p| p.parent())
                    {
                        let mut cmd = tokio::process::Command::new("git");
                        cmd.args(["-C", &repo_root.display().to_string(), "branch", "-D", branch])
                            .env_remove("GIT_DIR")
                            .env_remove("GIT_WORK_TREE");
                        let _ = oj_adapters::subprocess::run_with_timeout(
                            cmd,
                            oj_adapters::subprocess::GIT_WORKTREE_TIMEOUT,
                            "git branch delete",
                        )
                        .await;
                    }
                }
            }
        }
    }

    // Remove workspace directory (in case worktree remove left remnants)
    if workspace_path.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(workspace_path).await {
            tracing::warn!(
                path = %workspace_path.display(),
                error = %e,
                "failed to remove workspace directory (best-effort)"
            );
        }
    }
}

/// Create a git worktree at the given path.
async fn create_worktree(
    path: &std::path::Path,
    repo_root: Option<std::path::PathBuf>,
    branch: Option<String>,
    start_point: Option<String>,
) -> Result<(), String> {
    // Create parent directory
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create workspace parent dir: {}", e))?;
    }

    let repo_root = repo_root.ok_or("repo_root required for worktree workspace")?;
    let branch = branch.ok_or("branch required for worktree workspace")?;
    let start_point = start_point.unwrap_or_else(|| "HEAD".to_string());

    let path_str = path.display().to_string();
    let mut cmd = tokio::process::Command::new("git");
    cmd.args([
        "-C",
        &repo_root.display().to_string(),
        "worktree",
        "add",
        "-b",
        &branch,
        &path_str,
        &start_point,
    ])
    .env_remove("GIT_DIR")
    .env_remove("GIT_WORK_TREE");
    let output = oj_adapters::subprocess::run_with_timeout(
        cmd,
        oj_adapters::subprocess::GIT_WORKTREE_TIMEOUT,
        "git worktree add",
    )
    .await
    .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add failed: {}", stderr.trim()));
    }

    Ok(())
}

/// Create a plain directory workspace.
async fn create_folder(path: &std::path::Path) -> Result<(), String> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| format!("failed to create workspace dir: {}", e))
}
