// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Mutation handlers for state-changing requests.

use std::sync::Arc;

use parking_lot::Mutex;

use oj_adapters::subprocess::{run_with_timeout, GIT_WORKTREE_TIMEOUT, TMUX_TIMEOUT};
use oj_core::{AgentId, AgentRunId, Event, JobId, SessionId, ShortId, WorkspaceId};
use oj_runbook::Runbook;
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::{
    AgentEntry, CronEntry, JobEntry, Response, SessionEntry, WorkerEntry, WorkspaceEntry,
};

use super::ConnectionError;
use super::ListenCtx;

/// Emit an event via the event bus.
///
/// Maps send errors to `ConnectionError::WalError`.
pub(super) fn emit(event_bus: &EventBus, event: Event) -> Result<(), ConnectionError> {
    event_bus
        .send(event)
        .map(|_| ())
        .map_err(|_| ConnectionError::WalError)
}

/// Shared flags for prune operations.
pub(super) struct PruneFlags<'a> {
    pub all: bool,
    pub dry_run: bool,
    pub namespace: Option<&'a str>,
}

/// Best-effort cleanup of job log, breadcrumb, and associated agent files.
fn cleanup_job_files(logs_path: &std::path::Path, job_id: &str) {
    let log_file = oj_engine::log_paths::job_log_path(logs_path, job_id);
    let _ = std::fs::remove_file(&log_file);
    let crumb_file = oj_engine::log_paths::breadcrumb_path(logs_path, job_id);
    let _ = std::fs::remove_file(&crumb_file);
    cleanup_agent_files(logs_path, job_id);
}

/// Best-effort cleanup of agent log file and directory.
fn cleanup_agent_files(logs_path: &std::path::Path, agent_id: &str) {
    let agent_log = logs_path.join("agent").join(format!("{}.log", agent_id));
    let _ = std::fs::remove_file(&agent_log);
    let agent_dir = logs_path.join("agent").join(agent_id);
    let _ = std::fs::remove_dir_all(&agent_dir);
}

/// Handle a status request.
pub(super) fn handle_status(ctx: &ListenCtx) -> Response {
    let uptime_secs = ctx.start_time.elapsed().as_secs();
    let (jobs_active, sessions_active) = {
        let state = ctx.state.lock();
        let active = state.jobs.values().filter(|p| !p.is_terminal()).count();
        let sessions = state.sessions.len();
        (active, sessions)
    };
    let orphan_count = ctx.orphans.lock().len();

    Response::Status {
        uptime_secs,
        jobs_active,
        sessions_active,
        orphan_count,
    }
}

/// Handle a session send request.
pub(super) fn handle_session_send(
    ctx: &ListenCtx,
    id: String,
    input: String,
) -> Result<Response, ConnectionError> {
    let session_id = {
        let state_guard = ctx.state.lock();
        if state_guard.sessions.contains_key(&id) {
            Some(id.clone())
        } else {
            state_guard.jobs.get(&id).and_then(|p| p.session_id.clone())
        }
    };

    match session_id {
        Some(sid) => {
            emit(
                &ctx.event_bus,
                Event::SessionInput {
                    id: SessionId::new(sid),
                    input,
                },
            )?;
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
    ctx: &ListenCtx,
    id: &str,
) -> Result<Response, ConnectionError> {
    let session_id = {
        let state_guard = ctx.state.lock();
        if state_guard.sessions.contains_key(id) {
            Some(id.to_string())
        } else {
            None
        }
    };

    match session_id {
        Some(sid) => {
            // Kill the tmux session
            let mut cmd = tokio::process::Command::new("tmux");
            cmd.args(["kill-session", "-t", &sid]);
            let _ = run_with_timeout(cmd, TMUX_TIMEOUT, "tmux kill-session").await;

            // Emit SessionDeleted to clean up state
            emit(
                &ctx.event_bus,
                Event::SessionDeleted {
                    id: SessionId::new(sid),
                },
            )?;
            Ok(Response::Ok)
        }
        None => Ok(Response::Error {
            message: format!("Session not found: {}", id),
        }),
    }
}

/// Handle a job resume request.
///
/// Validates that the job exists in state or the orphan registry before
/// emitting the resume event. For orphaned jobs, emits synthetic events
/// to reconstruct the job in state, then resumes.
pub(super) fn handle_job_resume(
    ctx: &ListenCtx,
    id: String,
    message: Option<String>,
    vars: std::collections::HashMap<String, String>,
    kill: bool,
) -> Result<Response, ConnectionError> {
    // Check if job exists in state and get relevant info for validation
    let job_info = {
        let state_guard = ctx.state.lock();
        state_guard.get_job(&id).map(|p| {
            (
                p.id.clone(),
                p.kind.clone(),
                p.step.clone(),
                p.runbook_hash.clone(),
            )
        })
    };

    if let Some((job_id, job_kind, current_step, runbook_hash)) = job_info {
        // Validate agent steps require --message before emitting event
        if message.is_none() && current_step != "failed" {
            if let Err(err_msg) = validate_resume_message(
                &ctx.state,
                &job_id,
                &job_kind,
                &current_step,
                &runbook_hash,
            ) {
                return Ok(Response::Error { message: err_msg });
            }
        }

        emit(
            &ctx.event_bus,
            Event::JobResume {
                id: JobId::new(job_id),
                message,
                vars,
                kill,
            },
        )?;
        return Ok(Response::Ok);
    }

    // Not in state — check orphan registry
    let orphan = {
        let orphans_guard = ctx.orphans.lock();
        orphans_guard
            .iter()
            .find(|bc| bc.job_id == id || bc.job_id.starts_with(&id))
            .cloned()
    };

    let Some(orphan) = orphan else {
        return Ok(Response::Error {
            message: format!("job not found: {}", id),
        });
    };

    // Orphan found — check if the runbook is available for reconstruction
    if orphan.runbook_hash.is_empty() {
        return Ok(Response::Error {
            message: format!(
                "job {} is orphaned (state lost during daemon restart) and cannot be \
                 resumed: breadcrumb missing runbook hash (written by older daemon version). \
                 Dismiss with `oj job prune --orphans` and re-run the job.",
                orphan.job_id
            ),
        });
    }

    // Verify the runbook is in state (needed for step definitions during resume)
    let runbook_available = {
        let state_guard = ctx.state.lock();
        state_guard.runbooks.contains_key(&orphan.runbook_hash)
    };

    if !runbook_available {
        return Ok(Response::Error {
            message: format!(
                "job {} is orphaned (state lost during daemon restart) and cannot be \
                 resumed: runbook is no longer available. Dismiss with \
                 `oj job prune --orphans` and re-run the job.",
                orphan.job_id
            ),
        });
    }

    // Reconstruct the job by emitting synthetic events:
    // 1. JobCreated (at current_step as initial_step)
    // 2. JobAdvanced to "failed" (so resume resets to the right step)
    // 3. JobResume (the actual resume request)
    let orphan_id = orphan.job_id.clone();
    let job_id = JobId::new(&orphan_id);
    let cwd = orphan.cwd.or(orphan.workspace_root).unwrap_or_default();

    emit(
        &ctx.event_bus,
        Event::JobCreated {
            id: job_id.clone(),
            kind: orphan.kind,
            name: orphan.name,
            runbook_hash: orphan.runbook_hash,
            cwd,
            vars: orphan.vars,
            initial_step: orphan.current_step,
            created_at_epoch_ms: 0,
            namespace: orphan.project,
            cron_name: None,
        },
    )?;

    emit(
        &ctx.event_bus,
        Event::JobAdvanced {
            id: job_id.clone(),
            step: "failed".to_string(),
        },
    )?;

    emit(
        &ctx.event_bus,
        Event::JobResume {
            id: job_id,
            message,
            vars,
            kill,
        },
    )?;

    // Remove from orphan registry
    {
        let mut orphans_guard = ctx.orphans.lock();
        if let Some(idx) = orphans_guard.iter().position(|bc| bc.job_id == orphan_id) {
            orphans_guard.remove(idx);
        }
    }

    Ok(Response::Ok)
}

/// Handle a bulk job resume request (--all).
///
/// Resumes all non-terminal jobs that are in a resumable state:
/// waiting, failed, or pending. With `--kill`, also resumes running jobs.
pub(super) fn handle_job_resume_all(
    ctx: &ListenCtx,
    kill: bool,
) -> Result<Response, ConnectionError> {
    let (targets, skipped) = {
        let state_guard = ctx.state.lock();
        let mut targets: Vec<String> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();

        for job in state_guard.jobs.values() {
            if job.is_terminal() {
                continue;
            }

            if !kill {
                // Without --kill, only resume jobs in a resumable state
                if !job.step_status.is_waiting()
                    && !matches!(
                        job.step_status,
                        oj_core::StepStatus::Failed | oj_core::StepStatus::Pending
                    )
                {
                    skipped.push((
                        job.id.clone(),
                        format!("job is {:?} (use --kill to force)", job.step_status),
                    ));
                    continue;
                }
            }

            targets.push(job.id.clone());
        }

        (targets, skipped)
    };

    let mut resumed = Vec::new();
    for job_id in targets {
        emit(
            &ctx.event_bus,
            Event::JobResume {
                id: JobId::new(&job_id),
                message: None,
                vars: std::collections::HashMap::new(),
                kill,
            },
        )?;
        resumed.push(job_id);
    }

    Ok(Response::JobsResumed { resumed, skipped })
}

/// Handle a job cancel request.
pub(super) fn handle_job_cancel(
    ctx: &ListenCtx,
    ids: Vec<String>,
) -> Result<Response, ConnectionError> {
    let mut cancelled = Vec::new();
    let mut already_terminal = Vec::new();
    let mut not_found = Vec::new();

    for id in ids {
        let is_valid = {
            let state_guard = ctx.state.lock();
            state_guard.get_job(&id).map(|p| !p.is_terminal())
        };

        match is_valid {
            Some(true) => {
                emit(
                    &ctx.event_bus,
                    Event::JobCancel {
                        id: JobId::new(id.clone()),
                    },
                )?;
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

    Ok(Response::JobsCancelled {
        cancelled,
        already_terminal,
        not_found,
    })
}

/// Handle workspace drop requests.
pub(super) async fn handle_workspace_drop(
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

/// Handle an agent send request.
///
/// Resolves agent_id via (in order, first match wins):
/// 1. Exact agent_id match across ALL step_history entries (prefer latest)
/// 2. Job ID lookup → latest agent from ALL step_history entries
/// 3. Prefix match on agent_id across ALL step_history entries (prefer latest)
/// 4. Standalone agent_runs match
/// 5. Session liveness check (tmux has-session) before returning 'not found'
pub(super) async fn handle_agent_send(
    ctx: &ListenCtx,
    agent_id: String,
    message: String,
) -> Result<Response, ConnectionError> {
    let resolved_agent_id = {
        let state_guard = ctx.state.lock();

        // (1) Exact agent_id match across ALL step history, prefer latest
        let mut found: Option<String> = None;
        for job in state_guard.jobs.values() {
            for record in job.step_history.iter().rev() {
                if let Some(aid) = &record.agent_id {
                    if aid == &agent_id {
                        found = Some(aid.clone());
                        break;
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }

        // (2) Job ID lookup → latest agent from ALL step history
        if found.is_none() {
            if let Some(job) = state_guard.get_job(&agent_id) {
                for record in job.step_history.iter().rev() {
                    if let Some(aid) = &record.agent_id {
                        found = Some(aid.clone());
                        break;
                    }
                }
            }
        }

        // (3) Prefix match across ALL step history entries, prefer latest
        if found.is_none() {
            for job in state_guard.jobs.values() {
                for record in job.step_history.iter().rev() {
                    if let Some(aid) = &record.agent_id {
                        if aid.starts_with(&agent_id) {
                            found = Some(aid.clone());
                            break;
                        }
                    }
                }
                if found.is_some() {
                    break;
                }
            }
        }

        // (4) Standalone agent_runs match
        if found.is_none() {
            for ar in state_guard.agent_runs.values() {
                let ar_agent_id = ar.agent_id.as_deref().unwrap_or(&ar.id);
                if ar_agent_id == agent_id
                    || ar.id == agent_id
                    || ar_agent_id.starts_with(&agent_id)
                    || ar.id.starts_with(&agent_id)
                {
                    found = Some(ar_agent_id.to_string());
                    break;
                }
            }
        }

        found
    };

    if let Some(aid) = resolved_agent_id {
        emit(
            &ctx.event_bus,
            Event::AgentInput {
                agent_id: AgentId::new(aid),
                input: message,
            },
        )?;
        return Ok(Response::Ok);
    }

    // (5) Session liveness check: before returning 'not found', verify the
    // tmux session isn't still alive (recovery scenario where state is stale)
    let mut cmd = tokio::process::Command::new("tmux");
    cmd.args(["has-session", "-t", &agent_id]);
    let session_alive = run_with_timeout(cmd, TMUX_TIMEOUT, "tmux has-session")
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if session_alive {
        emit(
            &ctx.event_bus,
            Event::AgentInput {
                agent_id: AgentId::new(&agent_id),
                input: message,
            },
        )?;
        return Ok(Response::Ok);
    }

    Ok(Response::Error {
        message: format!("Agent not found: {}", agent_id),
    })
}

/// Handle job prune requests.
///
/// Removes terminal jobs (failed/cancelled/done) from state and
/// cleans up their log files. By default only prunes jobs older
/// than 12 hours; use `--all` to prune all terminal jobs.
pub(super) fn handle_job_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
    failed: bool,
    orphans: bool,
) -> Result<Response, ConnectionError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_threshold_ms = 12 * 60 * 60 * 1000; // 12 hours in ms

    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    // When --orphans is used alone, skip the normal terminal-job loop.
    // When combined with --all or --failed, run both.
    let prune_terminal = flags.all || failed || !orphans;

    if prune_terminal {
        let state_guard = ctx.state.lock();
        for job in state_guard.jobs.values() {
            // Filter by namespace when --project is specified
            if let Some(ns) = flags.namespace {
                if job.namespace != ns {
                    continue;
                }
            }

            if !job.is_terminal() {
                skipped += 1;
                continue;
            }

            // --failed flag: only prune failed jobs (skip done/cancelled)
            if failed && job.step != "failed" {
                skipped += 1;
                continue;
            }

            // Determine if this job skips the age check:
            // - --all: everything skips age check
            // - --failed: failed jobs skip age check
            // - cancelled jobs always skip age check (default behavior)
            let skip_age_check =
                flags.all || (failed && job.step == "failed") || job.step == "cancelled";

            if !skip_age_check {
                let created_at_ms = job
                    .step_history
                    .first()
                    .map(|r| r.started_at_ms)
                    .unwrap_or(0);
                if created_at_ms > 0 && now_ms.saturating_sub(created_at_ms) < age_threshold_ms {
                    skipped += 1;
                    continue;
                }
            }

            to_prune.push(JobEntry {
                id: job.id.clone(),
                name: job.name.clone(),
                step: job.step.clone(),
            });
        }
    }

    if !flags.dry_run {
        for entry in &to_prune {
            emit(
                &ctx.event_bus,
                Event::JobDeleted {
                    id: JobId::new(entry.id.clone()),
                },
            )?;
            cleanup_job_files(&ctx.logs_path, &entry.id);
        }
    }

    // When --orphans flag is set, collect orphaned jobs
    if orphans {
        let mut orphan_guard = ctx.orphans.lock();
        let drain_indices: Vec<usize> = (0..orphan_guard.len()).collect();
        for &i in drain_indices.iter().rev() {
            let bc = &orphan_guard[i];
            to_prune.push(JobEntry {
                id: bc.job_id.clone(),
                name: bc.name.clone(),
                step: "orphaned".to_string(),
            });
            if !flags.dry_run {
                let removed = orphan_guard.remove(i);
                cleanup_job_files(&ctx.logs_path, &removed.job_id);
            }
        }
    }

    Ok(Response::JobsPruned {
        pruned: to_prune,
        skipped,
    })
}

/// Handle agent prune requests.
///
/// Removes agent log files for agents belonging to terminal jobs
/// (failed/cancelled/done) and standalone agent runs in terminal state.
/// By default only prunes agents older than 24 hours; use `--all` to prune all.
pub(super) fn handle_agent_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_threshold_ms = 24 * 60 * 60 * 1000; // 24 hours in ms

    let mut to_prune = Vec::new();
    let mut job_ids_to_delete = Vec::new();
    let mut agent_run_ids_to_delete = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = ctx.state.lock();

        // (1) Collect agents from terminal jobs
        for job in state_guard.jobs.values() {
            if !job.is_terminal() {
                skipped += 1;
                continue;
            }

            // Check age via step history
            if !flags.all {
                let created_at_ms = job
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
            for record in &job.step_history {
                if let Some(agent_id) = &record.agent_id {
                    to_prune.push(AgentEntry {
                        agent_id: agent_id.clone(),
                        job_id: job.id.clone(),
                        step_name: record.name.clone(),
                    });
                }
            }

            job_ids_to_delete.push(job.id.clone());
        }

        // (2) Collect standalone agent runs in terminal state
        for agent_run in state_guard.agent_runs.values() {
            if !agent_run.is_terminal() {
                skipped += 1;
                continue;
            }

            // Check age
            if !flags.all
                && agent_run.created_at_ms > 0
                && now_ms.saturating_sub(agent_run.created_at_ms) < age_threshold_ms
            {
                skipped += 1;
                continue;
            }

            // Use agent_id if set, otherwise fall back to agent_run.id
            let agent_id = agent_run
                .agent_id
                .clone()
                .unwrap_or_else(|| agent_run.id.clone());

            to_prune.push(AgentEntry {
                agent_id,
                job_id: String::new(), // Empty for standalone agents
                step_name: agent_run.agent_name.clone(),
            });

            agent_run_ids_to_delete.push(agent_run.id.clone());
        }
    }

    if !flags.dry_run {
        // Delete the terminal jobs from state so agents no longer appear in `agent list`
        for job_id in &job_ids_to_delete {
            emit(
                &ctx.event_bus,
                Event::JobDeleted {
                    id: JobId::new(job_id.clone()),
                },
            )?;
            cleanup_job_files(&ctx.logs_path, job_id);
        }

        // Delete standalone agent runs from state
        for agent_run_id in &agent_run_ids_to_delete {
            emit(
                &ctx.event_bus,
                Event::AgentRunDeleted {
                    id: AgentRunId::new(agent_run_id),
                },
            )?;
        }

        for entry in &to_prune {
            cleanup_agent_files(&ctx.logs_path, &entry.agent_id);
        }
    }

    Ok(Response::AgentsPruned {
        pruned: to_prune,
        skipped,
    })
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
pub(super) async fn handle_workspace_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let state_dir = crate::env::state_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let workspaces_dir = state_dir.join("workspaces");
    workspace_prune_inner(&ctx.state, &ctx.event_bus, flags, &workspaces_dir).await
}

/// Inner implementation of workspace prune, parameterized by workspaces directory
/// for testability.
async fn workspace_prune_inner(
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

/// Handle worker prune requests.
///
/// Removes all stopped workers from state by emitting WorkerDeleted events.
/// Workers are either "running" or "stopped" — all stopped workers are eligible
/// for pruning with no age threshold.
pub(super) fn handle_worker_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = ctx.state.lock();
        for record in state_guard.workers.values() {
            // Filter by namespace if specified
            if let Some(ns) = flags.namespace {
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

    if !flags.dry_run {
        for entry in &to_prune {
            emit(
                &ctx.event_bus,
                Event::WorkerDeleted {
                    worker_name: entry.name.clone(),
                    namespace: entry.namespace.clone(),
                },
            )?;
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
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = ctx.state.lock();
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

    if !flags.dry_run {
        for entry in &to_prune {
            emit(
                &ctx.event_bus,
                Event::CronDeleted {
                    cron_name: entry.name.clone(),
                    namespace: entry.namespace.clone(),
                },
            )?;
        }
    }

    Ok(Response::CronsPruned {
        pruned: to_prune,
        skipped,
    })
}

/// Handle session prune requests.
///
/// Removes sessions whose associated job is terminal (done/failed/cancelled)
/// or missing from state. By default only prunes sessions older than 12 hours;
/// use `--all` to prune all orphaned sessions.
pub(super) async fn handle_session_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_threshold_ms = 12 * 60 * 60 * 1000; // 12 hours in ms

    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = ctx.state.lock();
        for session in state_guard.sessions.values() {
            // Get the namespace from the associated job
            let (namespace, job_is_terminal, job_created_at_ms) =
                match state_guard.jobs.get(&session.job_id) {
                    Some(job) => {
                        let created_at_ms = job
                            .step_history
                            .first()
                            .map(|r| r.started_at_ms)
                            .unwrap_or(0);
                        (job.namespace.clone(), job.is_terminal(), created_at_ms)
                    }
                    None => {
                        // Job missing from state - check if it's a standalone agent run
                        let agent_run = state_guard
                            .agent_runs
                            .values()
                            .find(|ar| ar.session_id.as_deref() == Some(session.id.as_str()));
                        match agent_run {
                            Some(ar) => (ar.namespace.clone(), ar.is_terminal(), ar.created_at_ms),
                            None => {
                                // Completely orphaned - no job or agent run
                                (String::new(), true, 0)
                            }
                        }
                    }
                };

            // Filter by namespace when --project is specified
            if let Some(ns) = flags.namespace {
                if namespace != ns {
                    continue;
                }
            }

            // Only prune sessions for terminal or missing jobs
            if !job_is_terminal {
                skipped += 1;
                continue;
            }

            // Check age unless --all is specified
            if !flags.all
                && job_created_at_ms > 0
                && now_ms.saturating_sub(job_created_at_ms) < age_threshold_ms
            {
                skipped += 1;
                continue;
            }

            to_prune.push(SessionEntry {
                id: session.id.clone(),
                job_id: session.job_id.clone(),
                namespace,
            });
        }
    }

    if !flags.dry_run {
        for entry in &to_prune {
            // Kill the tmux session (best effort)
            let _ = tokio::process::Command::new("tmux")
                .args(["kill-session", "-t", &entry.id])
                .output()
                .await;

            // Emit SessionDeleted to clean up state
            emit(
                &ctx.event_bus,
                Event::SessionDeleted {
                    id: SessionId::new(&entry.id),
                },
            )?;
        }
    }

    Ok(Response::SessionsPruned {
        pruned: to_prune,
        skipped,
    })
}

/// Handle an agent resume request.
///
/// Finds the agent by ID/prefix (or all dead agents when `all` is true),
/// optionally kills the tmux session, then emits JobResume to trigger
/// the engine's resume flow (which uses `--resume` to preserve conversation).
pub(super) async fn handle_agent_resume(
    ctx: &ListenCtx,
    agent_id: String,
    kill: bool,
    all: bool,
) -> Result<Response, ConnectionError> {
    // Collect (job_id, agent_id, session_id) tuples to resume
    // Use a scoped block to ensure lock is released before any await points
    let (targets, skipped) = {
        let state_guard = ctx.state.lock();
        let mut targets: Vec<(String, String, Option<String>)> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();

        if all {
            // Iterate all non-terminal jobs, find ones with agents
            for job in state_guard.jobs.values() {
                if job.is_terminal() {
                    continue;
                }
                // Get the current step's agent
                if let Some(record) = job.step_history.iter().rfind(|r| r.name == job.step) {
                    if let Some(ref aid) = record.agent_id {
                        if !kill {
                            // Without --kill, only resume agents that are
                            // escalated/waiting (dead session scenario)
                            if !job.step_status.is_waiting()
                                && !matches!(
                                    job.step_status,
                                    oj_core::StepStatus::Failed | oj_core::StepStatus::Pending
                                )
                            {
                                skipped.push((
                                    aid.clone(),
                                    format!("agent is {:?} (use --kill to force)", job.step_status),
                                ));
                                continue;
                            }
                        }
                        targets.push((job.id.clone(), aid.clone(), job.session_id.clone()));
                    }
                }
            }
        } else {
            // Find specific agent by ID or prefix
            let mut found = false;
            for job in state_guard.jobs.values() {
                for record in &job.step_history {
                    if let Some(ref aid) = record.agent_id {
                        if aid == &agent_id || aid.starts_with(&agent_id) {
                            if job.is_terminal() {
                                return Ok(Response::Error {
                                    message: format!(
                                        "job {} is already {} — cannot resume agent",
                                        job.id, job.step
                                    ),
                                });
                            }
                            targets.push((job.id.clone(), aid.clone(), job.session_id.clone()));
                            found = true;
                            break;
                        }
                    }
                }
                if found {
                    break;
                }
            }

            if !found {
                return Ok(Response::Error {
                    message: format!("agent not found: {}", agent_id),
                });
            }
        }

        (targets, skipped)
    };

    // If --kill is specified, kill the tmux sessions first
    if kill {
        for (_, _, session_id) in &targets {
            if let Some(sid) = session_id {
                // Kill the tmux session (ignore errors - session may already be dead)
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.args(["kill-session", "-t", sid]);
                let _ = run_with_timeout(cmd, TMUX_TIMEOUT, "tmux kill-session").await;

                // Emit SessionDeleted to clean up state
                let event = Event::SessionDeleted {
                    id: SessionId::new(sid),
                };
                let _ = ctx.event_bus.send(event);
            }
        }
    }

    let mut resumed = Vec::new();

    for (job_id, aid, _) in targets {
        emit(
            &ctx.event_bus,
            Event::JobResume {
                id: JobId::new(&job_id),
                message: None,
                vars: std::collections::HashMap::new(),
                kill,
            },
        )?;
        resumed.push(aid);
    }

    Ok(Response::AgentResumed { resumed, skipped })
}

/// Validate that agent steps have a message for resume.
///
/// Returns `Ok(())` if validation passes, or `Err(message)` with an error message
/// if the step is an agent step and no message was provided.
fn validate_resume_message(
    state: &Arc<Mutex<MaterializedState>>,
    job_id: &str,
    job_kind: &str,
    current_step: &str,
    runbook_hash: &str,
) -> Result<(), String> {
    // Get the stored runbook
    let stored = {
        let state_guard = state.lock();
        state_guard.runbooks.get(runbook_hash).cloned()
    };

    let Some(stored) = stored else {
        // If runbook is not found, let the engine handle it
        return Ok(());
    };

    // Parse the runbook
    let runbook: Runbook = match serde_json::from_value(stored.data) {
        Ok(rb) => rb,
        Err(_) => {
            // If we can't parse, let the engine handle it
            return Ok(());
        }
    };

    // Get the job and step definitions
    let Some(job_def) = runbook.get_job(job_kind) else {
        return Ok(());
    };
    let Some(step_def) = job_def.get_step(current_step) else {
        return Ok(());
    };

    // Check if it's an agent step
    if step_def.is_agent() {
        let short_id = job_id.short(12);
        return Err(format!(
            "agent steps require --message for resume. Example:\n  \
             oj job resume {} -m \"I fixed the import, try again\"",
            short_id
        ));
    }

    Ok(())
}

#[cfg(test)]
#[path = "mutations_tests.rs"]
mod tests;
