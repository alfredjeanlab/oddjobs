// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Mutation handlers for state-changing requests.

use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{Event, PipelineId, SessionId, WorkspaceId};
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::{Response, WorkspaceEntry};

use super::ConnectionError;

/// Handle a status request.
pub(super) fn handle_status(
    state: &Arc<Mutex<MaterializedState>>,
    start_time: std::time::Instant,
) -> Response {
    let uptime_secs = start_time.elapsed().as_secs();
    let (pipelines_active, sessions_active) = {
        let state = state.lock();
        let active = state
            .pipelines
            .values()
            .filter(|p| !p.is_terminal())
            .count();
        let sessions = state.sessions.len();
        (active, sessions)
    };

    Response::Status {
        uptime_secs,
        pipelines_active,
        sessions_active,
    }
}

/// Handle a session send request.
pub(super) fn handle_session_send(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    id: String,
    input: String,
) -> Result<Response, ConnectionError> {
    let session_id = {
        let state_guard = state.lock();
        if state_guard.sessions.contains_key(&id) {
            Some(id.clone())
        } else {
            state_guard
                .pipelines
                .get(&id)
                .and_then(|p| p.session_id.clone())
        }
    };

    match session_id {
        Some(sid) => {
            let event = Event::SessionInput {
                id: SessionId::new(sid),
                input,
            };
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;
            Ok(Response::Ok)
        }
        None => Ok(Response::Error {
            message: format!("Session not found: {}", id),
        }),
    }
}

/// Handle a pipeline resume request.
pub(super) fn handle_pipeline_resume(
    event_bus: &EventBus,
    id: String,
    message: Option<String>,
    vars: std::collections::HashMap<String, String>,
) -> Result<Response, ConnectionError> {
    let event = Event::PipelineResume {
        id: PipelineId::new(id),
        message,
        vars,
    };
    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;
    Ok(Response::Ok)
}

/// Handle a pipeline cancel request.
pub(super) fn handle_pipeline_cancel(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    ids: Vec<String>,
) -> Result<Response, ConnectionError> {
    let mut cancelled = Vec::new();
    let mut already_terminal = Vec::new();
    let mut not_found = Vec::new();

    for id in ids {
        let is_valid = {
            let state_guard = state.lock();
            state_guard.get_pipeline(&id).map(|p| !p.is_terminal())
        };

        match is_valid {
            Some(true) => {
                let event = Event::PipelineCancel {
                    id: PipelineId::new(id.clone()),
                };
                event_bus
                    .send(event)
                    .map_err(|_| ConnectionError::WalError)?;
                cancelled.push(id);
            }
            Some(false) => {
                already_terminal.push(id);
            }
            None => {
                not_found.push(id);
            }
        }
    }

    Ok(Response::PipelinesCancelled {
        cancelled,
        already_terminal,
        not_found,
    })
}

/// Handle workspace drop requests.
pub(super) async fn handle_workspace_drop(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    id: Option<&str>,
    failed_only: bool,
    drop_all: bool,
) -> Result<Response, ConnectionError> {
    let workspaces_to_drop: Vec<(String, std::path::PathBuf, Option<String>)> = {
        let state = state.lock();

        if let Some(id) = id {
            // Find workspace by exact match or prefix
            let matches: Vec<_> = state
                .workspaces
                .iter()
                .filter(|(k, _)| *k == id || k.starts_with(id))
                .collect();

            if matches.len() == 1 {
                vec![(
                    matches[0].0.clone(),
                    matches[0].1.path.clone(),
                    matches[0].1.branch.clone(),
                )]
            } else if matches.is_empty() {
                return Ok(Response::Error {
                    message: format!("workspace not found: {}", id),
                });
            } else {
                return Ok(Response::Error {
                    message: format!("ambiguous workspace ID '{}': {} matches", id, matches.len()),
                });
            }
        } else if failed_only {
            state
                .workspaces
                .iter()
                .filter(|(_, w)| matches!(w.status, oj_core::WorkspaceStatus::Failed { .. }))
                .map(|(id, w)| (id.clone(), w.path.clone(), w.branch.clone()))
                .collect()
        } else if drop_all {
            state
                .workspaces
                .iter()
                .map(|(id, w)| (id.clone(), w.path.clone(), w.branch.clone()))
                .collect()
        } else {
            return Ok(Response::Error {
                message: "specify a workspace ID, --failed, or --all".to_string(),
            });
        }
    };

    let dropped: Vec<WorkspaceEntry> = workspaces_to_drop
        .iter()
        .map(|(id, path, branch)| WorkspaceEntry {
            id: id.clone(),
            path: path.clone(),
            branch: branch.clone(),
        })
        .collect();

    // Emit delete events for each workspace
    for (id, _path, _branch) in workspaces_to_drop {
        let event = Event::WorkspaceDrop {
            id: WorkspaceId::new(id),
        };
        event_bus
            .send(event)
            .map_err(|_| ConnectionError::WalError)?;
    }

    Ok(Response::WorkspacesDropped { dropped })
}

/// Handle workspace prune requests.
///
/// Iterates `$OJ_STATE_DIR/workspaces/` children on the filesystem.
/// For each directory: if it has a `.git` file (indicating a git worktree),
/// best-effort `git worktree remove`; then `rm -rf` regardless.
pub(super) async fn handle_workspace_prune(
    all: bool,
    dry_run: bool,
) -> Result<Response, ConnectionError> {
    let state_dir = std::env::var("OJ_STATE_DIR").unwrap_or_else(|_| {
        format!(
            "{}/.local/state/oj",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let workspaces_dir = std::path::PathBuf::from(&state_dir).join("workspaces");

    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    // Read immediate children of the workspaces directory
    let entries = match tokio::fs::read_dir(&workspaces_dir).await {
        Ok(entries) => entries,
        Err(_) => {
            // Directory doesn't exist or isn't readable — nothing to prune
            return Ok(Response::WorkspacesPruned {
                pruned: Vec::new(),
                skipped: 0,
            });
        }
    };

    let now = std::time::SystemTime::now();
    let age_threshold = std::time::Duration::from_secs(24 * 60 * 60);

    let mut entries = entries;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Check age via directory mtime (skip if < 24h unless --all)
        if !all {
            if let Ok(metadata) = tokio::fs::metadata(&path).await {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age < age_threshold {
                            skipped += 1;
                            continue;
                        }
                    }
                }
            }
        }

        let id = entry.file_name().to_string_lossy().to_string();
        to_prune.push(WorkspaceEntry {
            id,
            path,
            branch: None,
        });
    }

    if !dry_run {
        for ws in &to_prune {
            // If the directory has a .git file (not directory), it's a git worktree
            let dot_git = ws.path.join(".git");
            if tokio::fs::symlink_metadata(&dot_git)
                .await
                .map(|m| m.is_file())
                .unwrap_or(false)
            {
                // Best-effort git worktree remove (ignore failures)
                let _ = tokio::process::Command::new("git")
                    .arg("worktree")
                    .arg("remove")
                    .arg("--force")
                    .arg(&ws.path)
                    .output()
                    .await;
            }

            // Remove directory regardless
            let _ = tokio::fs::remove_dir_all(&ws.path).await;
        }
    }

    Ok(Response::WorkspacesPruned {
        pruned: to_prune,
        skipped,
    })
}
