// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace event handlers.

use oj_core::{Event, OwnerId, WorkspaceStatus};

use super::helpers;
use super::types::{Workspace, WorkspaceType};
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::WorkspaceCreated { id, path, branch, owner, workspace_type } => {
            let ws_type = workspace_type
                .as_deref()
                .map(|s| match s {
                    "worktree" => WorkspaceType::Worktree,
                    _ => WorkspaceType::Folder,
                })
                .unwrap_or_default();

            // Update the job's workspace info if owner is a job
            if let OwnerId::Job(job_id) = owner {
                if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                    job.workspace_path = Some(path.clone());
                    job.workspace_id = Some(id.clone());
                }
            }

            state.workspaces.insert(
                id.to_string(),
                Workspace {
                    id: id.to_string(),
                    path: path.clone(),
                    branch: branch.clone(),
                    owner: owner.clone(),
                    status: WorkspaceStatus::Creating,
                    workspace_type: ws_type,
                    created_at_ms: helpers::epoch_ms_now(),
                },
            );
        }

        Event::WorkspaceReady { id } => {
            if let Some(workspace) = state.workspaces.get_mut(id.as_str()) {
                workspace.status = WorkspaceStatus::Ready;
            }
        }

        Event::WorkspaceFailed { id, reason } => {
            if let Some(workspace) = state.workspaces.get_mut(id.as_str()) {
                workspace.status = WorkspaceStatus::Failed { reason: reason.clone() };
            }
        }

        Event::WorkspaceDeleted { id } => {
            state.workspaces.remove(id.as_str());
        }

        _ => {}
    }
}
