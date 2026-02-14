// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! State reconciliation after daemon restart.
//!
//! Checks persisted state against live agent processes and reconnects
//! monitoring or triggers appropriate exit handling for each entity.
//! Uses the RuntimeRouter (via recover methods) to probe the correct adapter
//! based on persisted AgentRuntime — works for Local, Docker, and K8s agents.

use std::collections::HashSet;

use oj_core::{AgentId, CrewId, CrewStatus, Event, JobId, OwnerId};
use tracing::{info, warn};

use super::ReconcileCtx;

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod tests;

/// Reconcile persisted state with actual world state after daemon restart.
///
/// For each non-terminal entity, attempts to reconnect monitoring via the
/// RuntimeRouter. The router probes the correct adapter (Local, Docker, or K8s)
/// based on the persisted AgentRuntime hint. On failure, the agent is presumed
/// gone and an AgentGone event is emitted to trigger exit handling.
pub(crate) async fn reconcile_state(ctx: &ReconcileCtx) {
    let state = &ctx.state_snapshot;

    // Resume workers that were running before the daemon restarted.
    // Re-emitting RunbookLoaded + WorkerStarted (matching the manual start path)
    // recreates the in-memory WorkerState and triggers an initial queue poll so
    // the worker picks up where it left off.
    let running_workers: Vec<_> =
        state.workers.values().filter(|w| w.status == "running").collect();

    if !running_workers.is_empty() {
        info!("Resuming {} running workers", running_workers.len());
    }

    // Emit RunbookLoaded for each unique hash referenced by running workers.
    // This populates the in-process runbook cache so WorkerStarted handlers
    // can find the runbook without falling back to disk.
    let mut emitted_hashes = HashSet::new();
    for worker in &running_workers {
        if emitted_hashes.insert(worker.runbook_hash.clone()) {
            if let Some(stored) = state.runbooks.get(&worker.runbook_hash) {
                let _ = ctx
                    .event_tx
                    .send(Event::RunbookLoaded {
                        hash: worker.runbook_hash.clone(),
                        version: stored.version,
                        runbook: stored.data.clone(),
                    })
                    .await;
            }
        }
    }

    for worker in &running_workers {
        info!(worker = %worker.name, project = %worker.project, "resuming worker after daemon restart");
        let _ = ctx
            .event_tx
            .send(Event::WorkerStarted {
                worker: worker.name.clone(),
                project_path: worker.project_path.clone(),
                runbook_hash: worker.runbook_hash.clone(),
                queue: worker.queue.clone(),
                concurrency: worker.concurrency,
                project: worker.project.clone(),
            })
            .await;
    }

    // Resume crons that were running before the daemon restarted.
    let running_crons: Vec<_> = state.crons.values().filter(|c| c.status == "running").collect();

    if !running_crons.is_empty() {
        info!("Resuming {} running crons", running_crons.len());
    }

    for cron in &running_crons {
        info!(cron = %cron.name, project = %cron.project, "resuming cron after daemon restart");
        let _ = ctx
            .event_tx
            .send(Event::CronStarted {
                cron: cron.name.clone(),
                project_path: cron.project_path.clone(),
                runbook_hash: cron.runbook_hash.clone(),
                interval: cron.interval.clone(),
                target: cron.target.clone(),
                project: cron.project.clone(),
            })
            .await;
    }

    // Reconcile crew
    let non_terminal_runs: Vec<_> =
        state.crew.values().filter(|run| !run.status.is_terminal()).collect();

    if !non_terminal_runs.is_empty() {
        info!("Reconciling {} non-terminal crew", non_terminal_runs.len());
    }

    for run in &non_terminal_runs {
        // If the crew has no agent_id, the agent was never fully spawned
        // (daemon crashed before CrewStarted was persisted). Directly mark
        // it failed — we can't route through AgentExited/AgentGone events because
        // the handler verifies agent_id matches.
        let Some(ref agent_id_str) = run.agent_id else {
            warn!(crew_id = %run.id, "no agent_id, marking failed");
            let _ = ctx
                .event_tx
                .send(Event::CrewUpdated {
                    id: CrewId::from_string(&run.id),
                    status: CrewStatus::Failed,
                    reason: Some("no agent_id at recovery".to_string()),
                })
                .await;
            continue;
        };

        // Try to reconnect monitoring via the RuntimeRouter.
        // recover_standalone_agent registers the agent→crew mapping, then
        // calls RuntimeRouter::reconnect which probes the correct adapter
        // based on the persisted AgentRuntime hint.
        match ctx.runtime.recover_standalone_agent(run).await {
            Ok(()) => {
                info!(
                    crew_id = %run.id,
                    agent_id = agent_id_str,
                    "recovering: standalone agent still running, reconnected"
                );
            }
            Err(e) => {
                // Reconnect failed — agent is unreachable, presumed gone.
                // recover_standalone_agent already registered the agent→crew
                // mapping, so AgentGone handlers can find the owner.
                info!(
                    crew_id = %run.id,
                    agent_id = agent_id_str,
                    error = %e,
                    "recovering: standalone agent gone while daemon was down"
                );
                let agent_id = AgentId::from_string(agent_id_str);
                let crew_id = CrewId::from_string(&run.id);
                let _ = ctx
                    .event_tx
                    .send(Event::AgentGone {
                        id: agent_id,
                        owner: OwnerId::crew(crew_id),
                        exit_code: None,
                    })
                    .await;
            }
        }
    }

    // Reconcile jobs
    let non_terminal: Vec<_> = state.jobs.values().filter(|p| !p.is_terminal()).collect();

    if non_terminal.is_empty() {
        return;
    }

    info!("Reconciling {} non-terminal jobs", non_terminal.len());

    for job in &non_terminal {
        // Extract agent_id from step_history (stored when agent was spawned).
        // This must match the UUID used during spawn — using any other format
        // causes the handler's stale-event check to drop the event.
        let agent_id_str =
            job.step_history.iter().rfind(|r| r.name == job.step).and_then(|r| r.agent_id.clone());

        // Waiting jobs (escalated to human) still need their monitoring reconnected
        // so that agent state changes are detected after decision resolution.
        if job.step_status.is_waiting() {
            if agent_id_str.is_some() {
                info!(job_id = %job.id, "reconnecting monitoring for Waiting job");
                if let Err(e) = ctx.runtime.recover_agent(job).await {
                    warn!(
                        job_id = %job.id,
                        error = %e,
                        "failed to reconnect monitoring for Waiting job"
                    );
                }
            }
            continue;
        }

        let Some(ref aid) = agent_id_str else {
            warn!(job_id = %job.id, "no agent_id in step_history, marking failed");
            let _ = ctx
                .event_tx
                .send(Event::JobAdvanced {
                    id: JobId::from_string(job.id.clone()),
                    step: "failed".to_string(),
                })
                .await;
            continue;
        };

        // Try to reconnect monitoring via the RuntimeRouter.
        // recover_agent registers the agent→job mapping, then calls
        // RuntimeRouter::reconnect which probes the correct adapter
        // based on the persisted AgentRuntime hint.
        match ctx.runtime.recover_agent(job).await {
            Ok(()) => {
                info!(job_id = %job.id, agent_id = aid, "recovering: agent still running, reconnected");
            }
            Err(e) => {
                // Reconnect failed — agent is unreachable, presumed gone.
                // recover_agent already registered the agent→job mapping,
                // so AgentGone handlers can find the owner.
                info!(job_id = %job.id, agent_id = aid, error = %e, "recovering: agent gone while daemon was down");
                let agent_id = AgentId::from_string(aid);
                let job_id = JobId::from_string(job.id.clone());
                let _ = ctx
                    .event_tx
                    .send(Event::AgentGone {
                        id: agent_id,
                        owner: OwnerId::job(job_id),
                        exit_code: None,
                    })
                    .await;
            }
        }
    }
}
