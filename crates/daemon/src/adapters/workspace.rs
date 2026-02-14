// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace filesystem adapter.
//!
//! The executor handles state bookkeeping (WorkspaceCreated/Ready/Failed
//! events); the adapter is responsible only for filesystem work in a
//! background task, sending result events via `event_tx`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use oj_core::{Event, WorkspaceId};
use tokio::sync::mpsc;

/// Parameters for provisioning a workspace.
pub struct ProvisionRequest {
    pub workspace_id: WorkspaceId,
    pub path: PathBuf,
    pub is_worktree: bool,
    pub repo_root: Option<PathBuf>,
    pub branch: Option<String>,
    pub start_point: Option<String>,
}

/// Adapter for provisioning and cleaning up workspace filesystem resources.
///
/// The executor handles state bookkeeping (WorkspaceCreated/Ready/Failed
/// events); the adapter is responsible only for filesystem work in a
/// background task, sending result events via `event_tx`.
#[async_trait]
pub trait WorkspaceAdapter: Send + Sync {
    /// Provision a workspace directory or git worktree.
    ///
    /// Must send exactly one of `WorkspaceReady` or `WorkspaceFailed`
    /// via `event_tx`.
    async fn provision(&self, event_tx: mpsc::Sender<Event>, req: ProvisionRequest);

    /// Clean up workspace filesystem resources.
    ///
    /// Must send `WorkspaceDeleted` via `event_tx` when done.
    async fn cleanup(
        &self,
        event_tx: mpsc::Sender<Event>,
        workspace_id: WorkspaceId,
        path: PathBuf,
        branch: Option<String>,
    );
}

/// Local filesystem workspace adapter — creates git worktrees and plain folders.
pub struct LocalWorkspaceAdapter;

#[async_trait]
impl WorkspaceAdapter for LocalWorkspaceAdapter {
    async fn provision(&self, event_tx: mpsc::Sender<Event>, req: ProvisionRequest) {
        let result = if req.is_worktree {
            create_worktree(&req.path, req.repo_root, req.branch, req.start_point).await
        } else {
            create_folder(&req.path).await
        };

        let event = match result {
            Ok(()) => Event::WorkspaceReady { id: req.workspace_id },
            Err(reason) => Event::WorkspaceFailed { id: req.workspace_id, reason },
        };

        if let Err(e) = event_tx.send(event).await {
            tracing::error!("failed to send workspace event: {}", e);
        }
    }

    async fn cleanup(
        &self,
        event_tx: mpsc::Sender<Event>,
        workspace_id: WorkspaceId,
        path: PathBuf,
        branch: Option<String>,
    ) {
        delete_workspace_files(&path, &branch).await;

        let event = Event::WorkspaceDeleted { id: workspace_id };
        if let Err(e) = event_tx.send(event).await {
            tracing::error!("failed to send WorkspaceDeleted: {}", e);
        }
    }
}

/// Noop workspace adapter — logs and sends synthetic success events.
///
/// Used in remote-only mode (e.g. Kubernetes) where agent pods provision
/// their own code via init containers.
pub struct NoopWorkspaceAdapter;

#[async_trait]
impl WorkspaceAdapter for NoopWorkspaceAdapter {
    async fn provision(&self, event_tx: mpsc::Sender<Event>, req: ProvisionRequest) {
        tracing::info!(workspace_id = ?req.workspace_id, "skipping local workspace creation (remote-only)");
        let event = Event::WorkspaceReady { id: req.workspace_id };
        if let Err(e) = event_tx.send(event).await {
            tracing::error!("failed to send workspace event: {}", e);
        }
    }

    async fn cleanup(
        &self,
        event_tx: mpsc::Sender<Event>,
        workspace_id: WorkspaceId,
        _path: PathBuf,
        _branch: Option<String>,
    ) {
        tracing::info!(?workspace_id, "skipping local workspace deletion (remote-only)");
        let event = Event::WorkspaceDeleted { id: workspace_id };
        if let Err(e) = event_tx.send(event).await {
            tracing::error!("failed to send WorkspaceDeleted: {}", e);
        }
    }
}

/// Create a workspace adapter based on whether we're in remote-only mode.
pub fn workspace_adapter(remote_only: bool) -> Arc<dyn WorkspaceAdapter> {
    if remote_only {
        Arc::new(NoopWorkspaceAdapter)
    } else {
        Arc::new(LocalWorkspaceAdapter)
    }
}

// ---- Filesystem helpers (moved from engine/workspace.rs) ----

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
    let output = crate::adapters::subprocess::run_with_timeout(
        cmd,
        crate::adapters::subprocess::GIT_WORKTREE_TIMEOUT,
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
        let _ = crate::adapters::subprocess::run_with_timeout(
            cmd,
            crate::adapters::subprocess::GIT_WORKTREE_TIMEOUT,
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
                        let _ = crate::adapters::subprocess::run_with_timeout(
                            cmd,
                            crate::adapters::subprocess::GIT_WORKTREE_TIMEOUT,
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
