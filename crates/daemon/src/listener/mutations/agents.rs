// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use tokio_util::sync::CancellationToken;

use crate::adapters::agent::LocalAdapter;
use crate::storage::MaterializedState;
use oj_core::{AgentId, CrewId, Event, JobId};

use crate::protocol::{AgentEntry, Response};

use super::super::{ConnectionError, ListenCtx};
use super::{emit, PruneFlags};

/// Result of resolving an agent identifier to a concrete agent.
struct ResolvedAgent {
    /// The canonical agent identifier.
    agent_id: String,
    /// The job this agent belongs to (if it's a job-backed agent).
    job_id: Option<String>,
}

/// Resolve an agent identifier to a concrete agent via multi-step lookup:
///
/// 1. Exact agent_id match across all job step_history entries (prefer latest step)
/// 2. Job ID lookup → latest agent from step_history
/// 3. Prefix match on agent_id across all job step_history entries (prefer latest step)
/// 4. Standalone crew (exact match on agent_id/run_id, then prefix match)
fn resolve_agent(state: &MaterializedState, query: &str) -> Option<ResolvedAgent> {
    // (1) Exact agent_id match across ALL step history
    for job in state.jobs.values() {
        for record in job.step_history.iter().rev() {
            if let Some(aid) = &record.agent_id {
                if aid == query {
                    return Some(ResolvedAgent {
                        agent_id: aid.clone(),
                        job_id: Some(job.id.clone()),
                    });
                }
            }
        }
    }

    // (2) Job ID lookup → latest agent from step history
    if let Some(job) = state.get_job(query) {
        for record in job.step_history.iter().rev() {
            if let Some(aid) = &record.agent_id {
                return Some(ResolvedAgent { agent_id: aid.clone(), job_id: Some(job.id.clone()) });
            }
        }
    }

    // (3) Prefix match across ALL step history entries
    for job in state.jobs.values() {
        for record in job.step_history.iter().rev() {
            if let Some(aid) = &record.agent_id {
                if oj_core::id::prefix_matches(aid, query) {
                    return Some(ResolvedAgent {
                        agent_id: aid.clone(),
                        job_id: Some(job.id.clone()),
                    });
                }
            }
        }
    }

    // (4) Standalone crew match
    for run in state.crew.values() {
        let ar_agent_id = run.agent_id.as_deref().unwrap_or(&run.id);
        if ar_agent_id == query
            || run.id == query
            || oj_core::id::prefix_matches(ar_agent_id, query)
            || oj_core::id::prefix_matches(&run.id, query)
        {
            return Some(ResolvedAgent { agent_id: ar_agent_id.to_string(), job_id: None });
        }
    }

    None
}

/// Handle an agent kill request.
///
/// Resolves agent_id, kills the coop agent process. The polling task detects
/// the process is gone and triggers on_dead lifecycle.
pub(crate) async fn handle_agent_kill(
    ctx: &ListenCtx,
    agent_id: String,
) -> Result<Response, ConnectionError> {
    let resolved = {
        let state = ctx.state.lock();
        resolve_agent(&state, &agent_id)
    };

    match resolved {
        Some(r) => {
            LocalAdapter::kill_agent(&ctx.state_dir, &r.agent_id).await;
            Ok(Response::Ok)
        }
        None => Ok(Response::Error {
            message: format!("Agent not found or has no session: {}", agent_id),
        }),
    }
}

/// Handle an agent send request.
///
/// Resolves agent_id via the standard 4-step lookup, then falls back to
/// a coop liveness check before returning 'not found'.
pub(crate) async fn handle_agent_send(
    ctx: &ListenCtx,
    agent_id: String,
    message: String,
) -> Result<Response, ConnectionError> {
    let resolved = {
        let state = ctx.state.lock();
        resolve_agent(&state, &agent_id)
    };

    if let Some(r) = resolved {
        emit(
            &ctx.event_bus,
            Event::AgentInput { id: AgentId::from_string(r.agent_id), input: message },
        )?;
        return Ok(Response::Ok);
    }

    // Liveness check: before returning 'not found', verify the agent isn't
    // still alive (recovery scenario where state is stale)
    if LocalAdapter::check_alive(&ctx.state_dir, &agent_id).await {
        emit(
            &ctx.event_bus,
            Event::AgentInput { id: AgentId::from_string(&agent_id), input: message },
        )?;
        return Ok(Response::Ok);
    }

    Ok(Response::Error { message: format!("Agent not found: {}", agent_id) })
}

/// Handle an agent resume request.
///
/// Finds the agent by ID/prefix (or all dead agents when `all` is true),
/// optionally kills the agent process, then emits JobResume to trigger
/// the engine's resume flow (which uses `--resume` to preserve conversation).
pub(crate) async fn handle_agent_resume(
    ctx: &ListenCtx,
    agent_id: String,
    kill: bool,
    all: bool,
    cancel: &CancellationToken,
) -> Result<Response, ConnectionError> {
    // Collect (job_id, agent_id) tuples to resume
    // Use a scoped block to ensure lock is released before any await points
    let (targets, skipped) = {
        let state_guard = ctx.state.lock();
        let mut targets: Vec<(String, String)> = Vec::new();
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
                        if !kill && !super::is_resumable_status(&job.step_status) {
                            skipped.push((
                                aid.clone(),
                                format!("agent is {:?} (use --kill to force)", job.step_status),
                            ));
                            continue;
                        }
                        targets.push((job.id.clone(), aid.clone()));
                    }
                }
            }
        } else {
            // Find specific agent by ID or prefix via standard resolution
            match resolve_agent(&state_guard, &agent_id) {
                Some(r) => {
                    let job_id = match r.job_id {
                        Some(id) => id,
                        None => {
                            return Ok(Response::Error {
                                message: format!("agent not found: {}", agent_id),
                            });
                        }
                    };
                    if let Some(job) = state_guard.get_job(&job_id) {
                        if job.is_terminal() {
                            return Ok(Response::Error {
                                message: format!(
                                    "job {} is already {} — cannot resume agent",
                                    job.id, job.step
                                ),
                            });
                        }
                    }
                    targets.push((job_id, r.agent_id));
                }
                None => {
                    return Ok(Response::Error {
                        message: format!("agent not found: {}", agent_id),
                    });
                }
            }
        }

        (targets, skipped)
    };

    // If --kill is specified, kill the agent processes first
    if kill {
        for (_, aid) in &targets {
            if cancel.is_cancelled() {
                return Ok(Response::Error {
                    message: "cancelled: client disconnected".to_string(),
                });
            }
            LocalAdapter::kill_agent(&ctx.state_dir, aid).await;
        }
    }

    let mut resumed = Vec::new();

    for (job_id, aid) in targets {
        emit(
            &ctx.event_bus,
            Event::JobResume {
                id: JobId::from_string(&job_id),
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
/// (failed/cancelled/done) and crew in terminal state.
/// By default only prunes agents older than 24 hours; use `--all` to prune all.
pub(crate) fn handle_agent_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let now_ms = super::prune_helpers::now_millis();
    let age_ms = 24 * 60 * 60 * 1000; // 24 hours

    let mut to_prune = Vec::new();
    let mut job_ids_to_delete = Vec::new();
    let mut crew_ids_to_delete = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = ctx.state.lock();

        // (1) Collect agents from terminal jobs
        for job in state_guard.jobs.values() {
            if !job.is_terminal() || job.is_suspended() {
                skipped += 1;
                continue;
            }

            let created = job.step_history.first().map(|r| r.started_at_ms).unwrap_or(0);
            if super::prune_helpers::within_age_threshold(flags.all, now_ms, created, age_ms) {
                skipped += 1;
                continue;
            }

            for record in &job.step_history {
                if let Some(agent_id) = &record.agent_id {
                    to_prune.push(AgentEntry {
                        agent_id: oj_core::AgentId::from_string(agent_id),
                        owner: oj_core::JobId::from_string(&job.id).into(),
                        step_name: record.name.clone(),
                    });
                }
            }

            job_ids_to_delete.push(job.id.clone());
        }

        // (2) Collect crew in terminal state
        for run in state_guard.crew.values() {
            if !run.status.is_terminal() {
                skipped += 1;
                continue;
            }

            if super::prune_helpers::within_age_threshold(
                flags.all,
                now_ms,
                run.created_at_ms,
                age_ms,
            ) {
                skipped += 1;
                continue;
            }

            let agent_id = run.agent_id.clone().unwrap_or_else(|| run.id.clone());

            to_prune.push(AgentEntry {
                agent_id: oj_core::AgentId::from_string(&agent_id),
                owner: oj_core::CrewId::from_string(&run.id).into(),
                step_name: run.agent_name.clone(),
            });

            crew_ids_to_delete.push(run.id.clone());
        }
    }

    if !flags.dry_run {
        // Delete the terminal jobs from state so agents no longer appear in `agent list`
        for job_id in &job_ids_to_delete {
            emit(&ctx.event_bus, Event::JobDeleted { id: JobId::from_string(job_id.clone()) })?;
            super::prune_helpers::cleanup_job_files(&ctx.logs_path, job_id);
        }

        // Delete crew from state
        for crew_id in &crew_ids_to_delete {
            emit(&ctx.event_bus, Event::CrewDeleted { id: CrewId::from_string(crew_id) })?;
        }

        for entry in &to_prune {
            super::prune_helpers::cleanup_agent_files(&ctx.logs_path, &entry.agent_id);
        }
    }

    Ok(Response::AgentsPruned { pruned: to_prune, skipped })
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
