// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use oj_adapters::subprocess::{run_with_timeout, AGENT_KILL_TIMEOUT, TMUX_TIMEOUT};
use oj_core::{AgentId, AgentRunId, Event, JobId, SessionId};

use crate::protocol::{AgentEntry, Response};

use super::super::{ConnectionError, ListenCtx};
use super::{emit, PruneFlags};

/// Handle an agent kill request.
///
/// Resolves agent_id to a session ID via (in order, first match wins):
/// 1. Exact agent_id match across ALL step_history entries → job.session_id
/// 2. Job ID lookup → job.session_id
/// 3. Prefix match on agent_id across ALL step_history entries → job.session_id
/// 4. Standalone agent_runs match → agent_run.session_id
///
/// Then kills the tmux session and emits SessionDeleted. The agent watcher
/// polling loop will detect the session is gone and trigger on_dead lifecycle.
pub(crate) async fn handle_agent_kill(
    ctx: &ListenCtx,
    agent_id: String,
) -> Result<Response, ConnectionError> {
    let resolved_session = {
        let state_guard = ctx.state.lock();

        let mut found: Option<String> = None;

        // (1) Exact agent_id match across ALL step history → job.session_id
        for job in state_guard.jobs.values() {
            for record in job.step_history.iter().rev() {
                if let Some(aid) = &record.agent_id {
                    if aid == &agent_id {
                        found = job.session_id.clone();
                        break;
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }

        // (2) Job ID lookup → job.session_id
        if found.is_none() {
            if let Some(job) = state_guard.get_job(&agent_id) {
                if job.step_history.iter().any(|r| r.agent_id.is_some()) {
                    found = job.session_id.clone();
                }
            }
        }

        // (3) Prefix match across ALL step history entries → job.session_id
        if found.is_none() {
            for job in state_guard.jobs.values() {
                for record in job.step_history.iter().rev() {
                    if let Some(aid) = &record.agent_id {
                        if aid.starts_with(&agent_id) {
                            found = job.session_id.clone();
                            break;
                        }
                    }
                }
                if found.is_some() {
                    break;
                }
            }
        }

        // (4) Standalone agent_runs match → agent_run.session_id
        if found.is_none() {
            for ar in state_guard.agent_runs.values() {
                let ar_agent_id = ar.agent_id.as_deref().unwrap_or(&ar.id);
                if ar_agent_id == agent_id
                    || ar.id == agent_id
                    || ar_agent_id.starts_with(&agent_id)
                    || ar.id.starts_with(&agent_id)
                {
                    found = ar.session_id.clone();
                    break;
                }
            }
        }

        found
    };

    match resolved_session {
        Some(session_id) => {
            // Kill the tmux session (ignore errors — session may already be dead)
            let mut cmd = tokio::process::Command::new("tmux");
            cmd.args(["kill-session", "-t", &session_id]);
            let _ = run_with_timeout(cmd, AGENT_KILL_TIMEOUT, "tmux kill-session").await;

            // Emit SessionDeleted to clean up state
            emit(
                &ctx.event_bus,
                Event::SessionDeleted {
                    id: SessionId::new(&session_id),
                },
            )?;
            Ok(Response::Ok)
        }
        None => Ok(Response::Error {
            message: format!("Agent not found or has no session: {}", agent_id),
        }),
    }
}

/// Handle an agent send request.
///
/// Resolves agent_id via (in order, first match wins):
/// 1. Exact agent_id match across ALL step_history entries (prefer latest)
/// 2. Job ID lookup -> latest agent from ALL step_history entries
/// 3. Prefix match on agent_id across ALL step_history entries (prefer latest)
/// 4. Standalone agent_runs match
/// 5. Session liveness check (tmux has-session) before returning 'not found'
pub(crate) async fn handle_agent_send(
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

        // (2) Job ID lookup -> latest agent from ALL step history
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

/// Handle an agent resume request.
///
/// Finds the agent by ID/prefix (or all dead agents when `all` is true),
/// optionally kills the tmux session, then emits JobResume to trigger
/// the engine's resume flow (which uses `--resume` to preserve conversation).
pub(crate) async fn handle_agent_resume(
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
                let _ = run_with_timeout(cmd, AGENT_KILL_TIMEOUT, "tmux kill-session").await;

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

/// Handle agent prune requests.
///
/// Removes agent log files for agents belonging to terminal jobs
/// (failed/cancelled/done) and standalone agent runs in terminal state.
/// By default only prunes agents older than 24 hours; use `--all` to prune all.
pub(crate) fn handle_agent_prune(
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

            // Never prune agents from suspended jobs — preserved for resume
            if job.is_suspended() {
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
            super::prune_helpers::cleanup_job_files(&ctx.logs_path, job_id);
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
            super::prune_helpers::cleanup_agent_files(&ctx.logs_path, &entry.agent_id);
        }
    }

    Ok(Response::AgentsPruned {
        pruned: to_prune,
        skipped,
    })
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
