// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::sync::Arc;

use parking_lot::Mutex;

use oj_adapters::subprocess::{run_with_timeout, GIT_WORKTREE_TIMEOUT};
use oj_core::{Event, WorkspaceId};
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::{Response, WorkspaceEntry};

use super::super::{ConnectionError, ListenCtx};
use super::{emit, PruneFlags};

/// Handle workspace drop requests.
pub(crate) async fn handle_workspace_drop(
    ctx: &ListenCtx,
    id: Option<&str>,
    failed_only: bool,
    drop_all: bool,
) -> Result<Response, ConnectionError> {
    let workspaces_to_drop: Vec<(String, std::path::PathBuf, Option<String>)> = {
        let state = ctx.state.lock();

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
        emit(
            &ctx.event_bus,
            Event::WorkspaceDrop {
                id: WorkspaceId::new(id),
            },
        )?;
    }

    Ok(Response::WorkspacesDropped { dropped })
}

/// Handle workspace prune requests.
///
/// Two-phase prune:
/// 1. Iterates `$OJ_STATE_DIR/workspaces/` children on the filesystem.
///    For each directory: if it has a `.git` file (indicating a git worktree),
///    best-effort `git worktree remove`; then `rm -rf` regardless.
/// 2. Scans daemon state for orphaned workspace entries whose directories
///    no longer exist on the filesystem, and removes those from state.
///
/// Emits `WorkspaceDeleted` events to keep daemon state in sync.
pub(crate) async fn handle_workspace_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let state_dir = crate::env::state_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let workspaces_dir = state_dir.join("workspaces");
    workspace_prune_inner(&ctx.state, &ctx.event_bus, flags, &workspaces_dir).await
}

/// Inner implementation of workspace prune, parameterized by workspaces directory
/// for testability.
pub(crate) async fn workspace_prune_inner(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    flags: &PruneFlags<'_>,
    workspaces_dir: &std::path::Path,
) -> Result<Response, ConnectionError> {
    // When filtering by namespace, build a set of workspace IDs that match.
    // Namespace is derived from the workspace's owner (job or worker).
    // Workspaces with no determinable namespace (no owner, or owner not in state)
    // are included when the owner is missing (truly orphaned).
    let namespace_filter: Option<std::collections::HashSet<String>> = flags.namespace.map(|ns| {
        let state_guard = state.lock();
        state_guard
            .workspaces
            .iter()
            .filter(|(_, w)| {
                let workspace_ns = w.owner.as_ref().and_then(|owner| match owner {
                    oj_core::OwnerId::Job(job_id) => state_guard
                        .jobs
                        .get(job_id.as_str())
                        .map(|p| p.namespace.as_str()),
                    oj_core::OwnerId::AgentRun(ar_id) => state_guard
                        .agent_runs
                        .get(ar_id.as_str())
                        .map(|ar| ar.namespace.as_str()),
                });
                // Include if namespace matches OR if owner is not resolvable (orphaned)
                match workspace_ns {
                    Some(workspace_ns) => workspace_ns == ns,
                    None => true,
                }
            })
            .map(|(id, _)| id.clone())
            .collect()
    });

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
    let age_threshold = std::time::Duration::from_secs(12 * 60 * 60);

    let mut entries = entries;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let id = entry.file_name().to_string_lossy().to_string();

        // Filter by namespace when --project is specified
        if let Some(ref allowed_ids) = namespace_filter {
            if !allowed_ids.contains(&id) {
                continue;
            }
        }

        // Check age via directory mtime (skip if < 12h unless --all)
        if !flags.all {
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

        to_prune.push(WorkspaceEntry {
            id,
            path,
            branch: None,
        });
    }

    // Phase 2: Find orphaned state entries (in daemon state but directory missing)
    let orphaned: Vec<WorkspaceEntry> = {
        let state_guard = state.lock();
        let fs_pruned_ids: std::collections::HashSet<&str> =
            to_prune.iter().map(|ws| ws.id.as_str()).collect();

        state_guard
            .workspaces
            .iter()
            .filter(|(id, ws)| {
                // Skip if already in the filesystem prune list
                if fs_pruned_ids.contains(id.as_str()) {
                    return false;
                }
                // Apply namespace filter
                if let Some(ref allowed_ids) = namespace_filter {
                    if !allowed_ids.contains(id.as_str()) {
                        return false;
                    }
                }
                // Include if the directory no longer exists
                !ws.path.is_dir()
            })
            .map(|(id, ws)| WorkspaceEntry {
                id: id.clone(),
                path: ws.path.clone(),
                branch: ws.branch.clone(),
            })
            .collect()
    };
    to_prune.extend(orphaned);

    if !flags.dry_run {
        for ws in &to_prune {
            // If the directory exists, clean it up
            if ws.path.is_dir() {
                // If the directory has a .git file (not directory), it's a git worktree
                let dot_git = ws.path.join(".git");
                if tokio::fs::symlink_metadata(&dot_git)
                    .await
                    .map(|m| m.is_file())
                    .unwrap_or(false)
                {
                    // Best-effort git worktree remove (ignore failures).
                    // Run from within the worktree so git can locate the parent repo.
                    let mut cmd = tokio::process::Command::new("git");
                    cmd.arg("worktree")
                        .arg("remove")
                        .arg("--force")
                        .arg(&ws.path)
                        .current_dir(&ws.path);
                    let _ =
                        run_with_timeout(cmd, GIT_WORKTREE_TIMEOUT, "git worktree remove").await;
                }

                // Remove directory regardless
                let _ = tokio::fs::remove_dir_all(&ws.path).await;
            }

            // Emit WorkspaceDeleted to remove from daemon state
            emit(
                event_bus,
                Event::WorkspaceDeleted {
                    id: WorkspaceId::new(&ws.id),
                },
            )?;
        }
    }

    Ok(Response::WorkspacesPruned {
        pruned: to_prune,
        skipped,
    })
}
