// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session and workspace event handlers.

use oj_core::{Event, OwnerId, WorkspaceStatus};

use super::helpers;
use super::types::{Session, Workspace, WorkspaceType};
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::SessionCreated { id, owner } => {
            // Extract job_id from owner for Session record (for backwards compat)
            let job_id_str = match owner {
                OwnerId::Job(jid) => jid.to_string(),
                OwnerId::AgentRun(_) => String::new(),
            };
            state.sessions.insert(
                id.to_string(),
                Session {
                    id: id.to_string(),
                    job_id: job_id_str,
                },
            );
            // Update the job's or agent_run's session_id based on owner
            match owner {
                OwnerId::Job(job_id) => {
                    if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                        job.session_id = Some(id.to_string());
                    }
                }
                OwnerId::AgentRun(ar_id) => {
                    if let Some(agent_run) = state.agent_runs.get_mut(ar_id.as_str()) {
                        agent_run.session_id = Some(id.to_string());
                    }
                }
            }
            // Set session_id on matching agent records
            for rec in state.agents.values_mut() {
                if rec.owner == *owner && rec.session_id.is_none() {
                    rec.session_id = Some(id.to_string());
                }
            }
        }

        Event::SessionDeleted { id } => {
            state.sessions.remove(id.as_str());

            // Clear job.session_id if it references the deleted session
            for job in state.jobs.values_mut() {
                if job.session_id.as_deref() == Some(id.as_str()) {
                    job.session_id = None;
                }
            }

            // Clear agent_run.session_id if it references the deleted session
            for agent_run in state.agent_runs.values_mut() {
                if agent_run.session_id.as_deref() == Some(id.as_str()) {
                    agent_run.session_id = None;
                }
            }

            // Clear agent record session_id if it references the deleted session
            for rec in state.agents.values_mut() {
                if rec.session_id.as_deref() == Some(id.as_str()) {
                    rec.session_id = None;
                }
            }
        }

        Event::WorkspaceCreated {
            id,
            path,
            branch,
            owner,
            workspace_type,
        } => {
            let ws_type = workspace_type
                .as_deref()
                .map(|s| match s {
                    "worktree" => WorkspaceType::Worktree,
                    _ => WorkspaceType::Folder,
                })
                .unwrap_or_default();

            // Update the job's workspace info if owner is a job
            if let Some(OwnerId::Job(job_id)) = owner {
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
                workspace.status = WorkspaceStatus::Failed {
                    reason: reason.clone(),
                };
            }
        }

        Event::WorkspaceDeleted { id } => {
            state.workspaces.remove(id.as_str());
        }

        _ => {}
    }
}
