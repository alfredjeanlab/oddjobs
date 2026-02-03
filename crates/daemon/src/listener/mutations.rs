// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Mutation handlers for state-changing requests.

use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{AgentId, Event, PipelineId, SessionId, WorkspaceId};
use oj_engine::breadcrumb::Breadcrumb;
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::{
    AgentEntry, CronEntry, PipelineEntry, Response, WorkerEntry, WorkspaceEntry,
};

use super::ConnectionError;

/// Handle a status request.
pub(super) fn handle_status(
    state: &Arc<Mutex<MaterializedState>>,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
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
    let orphan_count = orphans.lock().len();

    Response::Status {
        uptime_secs,
        pipelines_active,
        sessions_active,
        orphan_count,
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

/// Handle a session kill request.
///
/// Validates that the session exists, kills the tmux session, and emits
/// a SessionDeleted event to clean up state.
pub(super) async fn handle_session_kill(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    id: &str,
) -> Result<Response, ConnectionError> {
    let session_id = {
        let state_guard = state.lock();
        if state_guard.sessions.contains_key(id) {
            Some(id.to_string())
        } else {
            None
        }
    };

    match session_id {
        Some(sid) => {
            // Kill the tmux session
            let _ = tokio::process::Command::new("tmux")
                .args(["kill-session", "-t", &sid])
                .output()
                .await;

            // Emit SessionDeleted to clean up state
            let event = Event::SessionDeleted {
                id: SessionId::new(sid),
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
///
/// Validates that the pipeline exists in state or the orphan registry before
/// emitting the resume event. For orphaned pipelines, emits synthetic events
/// to reconstruct the pipeline in state, then resumes.
pub(super) fn handle_pipeline_resume(
    state: &Arc<Mutex<MaterializedState>>,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    event_bus: &EventBus,
    id: String,
    message: Option<String>,
    vars: std::collections::HashMap<String, String>,
) -> Result<Response, ConnectionError> {
    // Check if pipeline exists in state
    let found_in_state = {
        let state_guard = state.lock();
        state_guard.get_pipeline(&id).is_some()
    };

    if found_in_state {
        let event = Event::PipelineResume {
            id: PipelineId::new(id),
            message,
            vars,
        };
        event_bus
            .send(event)
            .map_err(|_| ConnectionError::WalError)?;
        return Ok(Response::Ok);
    }

    // Not in state — check orphan registry
    let orphan = {
        let orphans_guard = orphans.lock();
        orphans_guard
            .iter()
            .find(|bc| bc.pipeline_id == id || bc.pipeline_id.starts_with(&id))
            .cloned()
    };

    let Some(orphan) = orphan else {
        return Ok(Response::Error {
            message: format!("pipeline not found: {}", id),
        });
    };

    // Orphan found — check if the runbook is available for reconstruction
    if orphan.runbook_hash.is_empty() {
        return Ok(Response::Error {
            message: format!(
                "pipeline {} is orphaned (state lost during daemon restart) and cannot be \
                 resumed: breadcrumb missing runbook hash (written by older daemon version). \
                 Dismiss with `oj pipeline prune --orphans` and re-run the pipeline.",
                orphan.pipeline_id
            ),
        });
    }

    // Verify the runbook is in state (needed for step definitions during resume)
    let runbook_available = {
        let state_guard = state.lock();
        state_guard.runbooks.contains_key(&orphan.runbook_hash)
    };

    if !runbook_available {
        return Ok(Response::Error {
            message: format!(
                "pipeline {} is orphaned (state lost during daemon restart) and cannot be \
                 resumed: runbook is no longer available. Dismiss with \
                 `oj pipeline prune --orphans` and re-run the pipeline.",
                orphan.pipeline_id
            ),
        });
    }

    // Reconstruct the pipeline by emitting synthetic events:
    // 1. PipelineCreated (at current_step as initial_step)
    // 2. PipelineAdvanced to "failed" (so resume resets to the right step)
    // 3. PipelineResume (the actual resume request)
    let orphan_id = orphan.pipeline_id.clone();
    let pipeline_id = PipelineId::new(&orphan_id);
    let cwd = orphan.cwd.or(orphan.workspace_root).unwrap_or_default();

    event_bus
        .send(Event::PipelineCreated {
            id: pipeline_id.clone(),
            kind: orphan.kind,
            name: orphan.name,
            runbook_hash: orphan.runbook_hash,
            cwd,
            vars: orphan.vars,
            initial_step: orphan.current_step,
            created_at_epoch_ms: 0,
            namespace: orphan.project,
        })
        .map_err(|_| ConnectionError::WalError)?;

    event_bus
        .send(Event::PipelineAdvanced {
            id: pipeline_id.clone(),
            step: "failed".to_string(),
        })
        .map_err(|_| ConnectionError::WalError)?;

    event_bus
        .send(Event::PipelineResume {
            id: pipeline_id,
            message,
            vars,
        })
        .map_err(|_| ConnectionError::WalError)?;

    // Remove from orphan registry
    {
        let mut orphans_guard = orphans.lock();
        if let Some(idx) = orphans_guard
            .iter()
            .position(|bc| bc.pipeline_id == orphan_id)
        {
            orphans_guard.remove(idx);
        }
    }

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

/// Handle an agent send request.
///
/// Resolves agent_id via:
/// 1. Direct match on pipeline agent_id (from step_history)
/// 2. Pipeline ID lookup → current step's agent_id
/// 3. Prefix match on either
pub(super) fn handle_agent_send(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    agent_id: String,
    message: String,
) -> Result<Response, ConnectionError> {
    let resolved_agent_id = {
        let state_guard = state.lock();

        // 1. Check if any pipeline has an agent with this exact ID or prefix
        let mut found: Option<String> = None;
        for pipeline in state_guard.pipelines.values() {
            if let Some(record) = pipeline.step_history.last() {
                if let Some(aid) = &record.agent_id {
                    if aid == &agent_id || aid.starts_with(&agent_id) {
                        found = Some(aid.clone());
                        break;
                    }
                }
            }
        }

        // 2. If not found by agent_id, try as pipeline ID → active agent
        if found.is_none() {
            if let Some(pipeline) = state_guard.get_pipeline(&agent_id) {
                if let Some(record) = pipeline.step_history.last() {
                    found = record.agent_id.clone();
                }
            }
        }

        found
    };

    match resolved_agent_id {
        Some(aid) => {
            let event = Event::AgentInput {
                agent_id: AgentId::new(aid),
                input: message,
            };
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;
            Ok(Response::Ok)
        }
        None => Ok(Response::Error {
            message: format!("Agent not found: {}", agent_id),
        }),
    }
}

/// Handle pipeline prune requests.
///
/// Removes terminal pipelines (failed/cancelled/done) from state and
/// cleans up their log files. By default only prunes pipelines older
/// than 12 hours; use `--all` to prune all terminal pipelines.
// TODO(refactor): group prune flags into a shared struct with handle_agent_prune/handle_workspace_prune
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_pipeline_prune(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    logs_path: &std::path::Path,
    orphans_registry: &Arc<Mutex<Vec<oj_engine::breadcrumb::Breadcrumb>>>,
    all: bool,
    failed: bool,
    orphans: bool,
    dry_run: bool,
    namespace: Option<&str>,
) -> Result<Response, ConnectionError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_threshold_ms = 12 * 60 * 60 * 1000; // 12 hours in ms

    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    // When --orphans is used alone, skip the normal terminal-pipeline loop.
    // When combined with --all or --failed, run both.
    let prune_terminal = all || failed || !orphans;

    if prune_terminal {
        let state_guard = state.lock();
        for pipeline in state_guard.pipelines.values() {
            // Filter by namespace when --project is specified
            if let Some(ns) = namespace {
                if pipeline.namespace != ns {
                    continue;
                }
            }

            if !pipeline.is_terminal() {
                skipped += 1;
                continue;
            }

            // --failed flag: only prune failed pipelines (skip done/cancelled)
            if failed && pipeline.step != "failed" {
                skipped += 1;
                continue;
            }

            // Determine if this pipeline skips the age check:
            // - --all: everything skips age check
            // - --failed: failed pipelines skip age check
            // - cancelled pipelines always skip age check (default behavior)
            let skip_age_check =
                all || (failed && pipeline.step == "failed") || pipeline.step == "cancelled";

            if !skip_age_check {
                let created_at_ms = pipeline
                    .step_history
                    .first()
                    .map(|r| r.started_at_ms)
                    .unwrap_or(0);
                if created_at_ms > 0 && now_ms.saturating_sub(created_at_ms) < age_threshold_ms {
                    skipped += 1;
                    continue;
                }
            }

            to_prune.push(PipelineEntry {
                id: pipeline.id.clone(),
                name: pipeline.name.clone(),
                step: pipeline.step.clone(),
            });
        }
    }

    if !dry_run {
        for entry in &to_prune {
            // Emit PipelineDeleted event to remove from state
            let event = Event::PipelineDeleted {
                id: PipelineId::new(entry.id.clone()),
            };
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;

            // Best-effort cleanup of pipeline log and breadcrumb files
            let log_file = oj_engine::log_paths::pipeline_log_path(logs_path, &entry.id);
            let _ = std::fs::remove_file(&log_file);
            let crumb_file = oj_engine::log_paths::breadcrumb_path(logs_path, &entry.id);
            let _ = std::fs::remove_file(&crumb_file);

            // Best-effort cleanup of agent log files for this pipeline's steps
            // Agent logs are at logs_path/agent/<agent_id>.log
            // The pipeline ID is used as agent_id for agent steps
            let agent_log = logs_path.join("agent").join(format!("{}.log", entry.id));
            let _ = std::fs::remove_file(&agent_log);
            let agent_dir = logs_path.join("agent").join(&entry.id);
            let _ = std::fs::remove_dir_all(&agent_dir);
        }
    }

    // When --orphans flag is set, collect orphaned pipelines
    if orphans {
        let mut orphan_guard = orphans_registry.lock();
        let drain_indices: Vec<usize> = (0..orphan_guard.len()).collect();
        for &i in drain_indices.iter().rev() {
            let bc = &orphan_guard[i];
            to_prune.push(PipelineEntry {
                id: bc.pipeline_id.clone(),
                name: bc.name.clone(),
                step: "orphaned".to_string(),
            });
            if !dry_run {
                let removed = orphan_guard.remove(i);
                // Delete breadcrumb file
                let crumb = oj_engine::log_paths::breadcrumb_path(logs_path, &removed.pipeline_id);
                let _ = std::fs::remove_file(&crumb);
                // Delete pipeline log
                let log_file =
                    oj_engine::log_paths::pipeline_log_path(logs_path, &removed.pipeline_id);
                let _ = std::fs::remove_file(&log_file);
                // Delete agent logs/dirs
                let agent_log = logs_path
                    .join("agent")
                    .join(format!("{}.log", removed.pipeline_id));
                let _ = std::fs::remove_file(&agent_log);
                let agent_dir = logs_path.join("agent").join(&removed.pipeline_id);
                let _ = std::fs::remove_dir_all(&agent_dir);
            }
        }
    }

    Ok(Response::PipelinesPruned {
        pruned: to_prune,
        skipped,
    })
}

/// Handle agent prune requests.
///
/// Removes agent log files for agents belonging to terminal pipelines
/// (failed/cancelled/done). By default only prunes agents from pipelines
/// older than 24 hours; use `--all` to prune all.
pub(super) fn handle_agent_prune(
    state: &Arc<Mutex<MaterializedState>>,
    logs_path: &std::path::Path,
    all: bool,
    dry_run: bool,
) -> Result<Response, ConnectionError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_threshold_ms = 24 * 60 * 60 * 1000; // 24 hours in ms

    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = state.lock();
        for pipeline in state_guard.pipelines.values() {
            if !pipeline.is_terminal() {
                skipped += 1;
                continue;
            }

            // Check age via step history
            if !all {
                let created_at_ms = pipeline
                    .step_history
                    .first()
                    .map(|r| r.started_at_ms)
                    .unwrap_or(0);
                if created_at_ms > 0 && now_ms.saturating_sub(created_at_ms) < age_threshold_ms {
                    skipped += 1;
                    continue;
                }
            }

            // Collect agents from step history
            for record in &pipeline.step_history {
                if let Some(agent_id) = &record.agent_id {
                    to_prune.push(AgentEntry {
                        agent_id: agent_id.clone(),
                        pipeline_id: pipeline.id.clone(),
                        step_name: record.name.clone(),
                    });
                }
            }
        }
    }

    if !dry_run {
        for entry in &to_prune {
            // Best-effort cleanup of agent log file
            let agent_log = logs_path
                .join("agent")
                .join(format!("{}.log", entry.agent_id));
            let _ = std::fs::remove_file(&agent_log);

            // Best-effort cleanup of agent log directory
            let agent_dir = logs_path.join("agent").join(&entry.agent_id);
            let _ = std::fs::remove_dir_all(&agent_dir);
        }
    }

    Ok(Response::AgentsPruned {
        pruned: to_prune,
        skipped,
    })
}

/// Handle workspace prune requests.
///
/// Iterates `$OJ_STATE_DIR/workspaces/` children on the filesystem.
/// For each directory: if it has a `.git` file (indicating a git worktree),
/// best-effort `git worktree remove`; then `rm -rf` regardless.
pub(super) async fn handle_workspace_prune(
    state: &Arc<Mutex<MaterializedState>>,
    all: bool,
    dry_run: bool,
    namespace: Option<&str>,
) -> Result<Response, ConnectionError> {
    let state_dir = std::env::var("OJ_STATE_DIR").unwrap_or_else(|_| {
        format!(
            "{}/.local/state/oj",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let workspaces_dir = std::path::PathBuf::from(&state_dir).join("workspaces");

    // When filtering by namespace, build a set of workspace IDs that match.
    // Namespace is derived from the workspace's owner (pipeline or worker).
    let namespace_filter: Option<std::collections::HashSet<String>> = namespace.map(|ns| {
        let state_guard = state.lock();
        state_guard
            .workspaces
            .iter()
            .filter(|(_, w)| {
                w.owner
                    .as_deref()
                    .and_then(|owner| {
                        state_guard
                            .pipelines
                            .get(owner)
                            .map(|p| p.namespace.as_str())
                            .or_else(|| {
                                state_guard
                                    .workers
                                    .get(owner)
                                    .map(|wr| wr.namespace.as_str())
                            })
                    })
                    .unwrap_or_default()
                    == ns
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
                // Best-effort git worktree remove (ignore failures).
                // Run from within the worktree so git can locate the parent repo.
                let _ = tokio::process::Command::new("git")
                    .arg("worktree")
                    .arg("remove")
                    .arg("--force")
                    .arg(&ws.path)
                    .current_dir(&ws.path)
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

/// Handle worker prune requests.
///
/// Removes all stopped workers from state by emitting WorkerDeleted events.
/// Workers are either "running" or "stopped" — all stopped workers are eligible
/// for pruning with no age threshold.
pub(super) fn handle_worker_prune(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    _all: bool,
    dry_run: bool,
    namespace: Option<&str>,
) -> Result<Response, ConnectionError> {
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = state.lock();
        for record in state_guard.workers.values() {
            // Filter by namespace if specified
            if let Some(ns) = namespace {
                if record.namespace != ns {
                    continue;
                }
            }
            if record.status != "stopped" {
                skipped += 1;
                continue;
            }
            to_prune.push(WorkerEntry {
                name: record.name.clone(),
                namespace: record.namespace.clone(),
            });
        }
    }

    if !dry_run {
        for entry in &to_prune {
            let event = Event::WorkerDeleted {
                worker_name: entry.name.clone(),
                namespace: entry.namespace.clone(),
            };
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;
        }
    }

    Ok(Response::WorkersPruned {
        pruned: to_prune,
        skipped,
    })
}

/// Handle cron prune requests.
///
/// Removes all stopped crons from state by emitting CronDeleted events.
/// Crons are either "running" or "stopped" — all stopped crons are eligible
/// for pruning with no age threshold.
pub(super) fn handle_cron_prune(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    _all: bool,
    dry_run: bool,
) -> Result<Response, ConnectionError> {
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = state.lock();
        for record in state_guard.crons.values() {
            if record.status != "stopped" {
                skipped += 1;
                continue;
            }
            to_prune.push(CronEntry {
                name: record.name.clone(),
                namespace: record.namespace.clone(),
            });
        }
    }

    if !dry_run {
        for entry in &to_prune {
            let event = Event::CronDeleted {
                cron_name: entry.name.clone(),
                namespace: entry.namespace.clone(),
            };
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;
        }
    }

    Ok(Response::CronsPruned {
        pruned: to_prune,
        skipped,
    })
}

#[cfg(test)]
#[path = "mutations_tests.rs"]
mod tests;
