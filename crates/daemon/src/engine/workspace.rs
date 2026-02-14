// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace effect execution (create/delete worktrees and folders).

pub(crate) use crate::adapters::workspace::CreateRequest;

use crate::adapters::WorkspaceAdapter;
use crate::engine::executor::ExecuteError;
use crate::storage::MaterializedState;
use oj_core::Event;

use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Execute a CreateWorkspace effect.
///
/// Immediately records the workspace in state and spawns a background task
/// for the filesystem work (worktree add or mkdir). Returns the
/// `WorkspaceCreated` event for WAL persistence.
pub(crate) async fn create(
    state: &Arc<Mutex<MaterializedState>>,
    event_tx: &mpsc::Sender<Event>,
    workspace: &Arc<dyn WorkspaceAdapter>,
    req: CreateRequest,
) -> Result<Option<Event>, ExecuteError> {
    // Phase 1: Create workspace record immediately so job.workspace_path is set.
    let provision_req = req.clone();
    let create_event = Event::WorkspaceCreated {
        id: req.workspace_id,
        path: req.path,
        branch: req.branch,
        owner: req.owner,
        workspace_type: req.workspace_type,
    };
    {
        let mut state = state.lock();
        state.apply_event(&create_event);
    }

    // Phase 2: Spawn background task for filesystem work.
    let event_tx = event_tx.clone();
    let workspace = Arc::clone(workspace);
    tokio::spawn(async move {
        workspace.provision(event_tx, provision_req).await;
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
    workspace: &Arc<dyn WorkspaceAdapter>,
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
    let workspace = Arc::clone(workspace);
    tokio::spawn(async move {
        workspace.cleanup(event_tx, workspace_id, workspace_path, workspace_branch).await;
    });

    Ok(None)
}
