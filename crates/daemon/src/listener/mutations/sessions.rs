// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use oj_adapters::subprocess::{run_with_timeout, TMUX_TIMEOUT};
use oj_core::{Event, SessionId};

use crate::protocol::{Response, SessionEntry};

use super::super::{ConnectionError, ListenCtx};
use super::{emit, PruneFlags};

/// Handle a session send request.
pub(crate) fn handle_session_send(
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
                    input: format!("{}\n", input),
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
pub(crate) async fn handle_session_kill(
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

/// Handle session prune requests.
///
/// Removes sessions whose associated job is terminal (done/failed/cancelled)
/// or missing from state. By default only prunes sessions older than 12 hours;
/// use `--all` to prune all orphaned sessions.
pub(crate) async fn handle_session_prune(
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
