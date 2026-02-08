// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! State reconciliation after daemon restart.
//!
//! Checks persisted state against actual tmux sessions and reconnects
//! monitoring or triggers appropriate exit handling for each entity.

use std::collections::HashSet;

use oj_adapters::SessionAdapter;
use oj_core::{AgentId, AgentRunId, AgentRunStatus, Event, JobId, OwnerId, SessionId};
use tracing::{info, warn};

use super::ReconcileCtx;

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod tests;

/// Reconcile sessions with actual tmux state after daemon restart.
///
/// Builds a set of session IDs referenced by active (non-prunable) entities,
/// then cleans up any sessions not in that set. Only `done` and `cancelled`
/// jobs are considered finished — `failed` jobs are kept because they may be
/// resumed.
async fn reconcile_sessions(ctx: &ReconcileCtx) {
    let state = &ctx.state_snapshot;
    if state.sessions.is_empty() {
        return;
    }

    // Build set of session IDs referenced by resumable entities.
    // Only done/cancelled jobs are truly finished; failed jobs may resume.
    let mut in_use: HashSet<&str> = HashSet::new();
    for job in state.jobs.values() {
        if job.step != "done" && job.step != "cancelled" {
            if let Some(sid) = &job.session_id {
                in_use.insert(sid.as_str());
            }
        }
    }
    for ar in state.agent_runs.values() {
        if !ar.is_terminal() {
            if let Some(sid) = &ar.session_id {
                in_use.insert(sid.as_str());
            }
        }
    }

    // Prune sessions not in use
    let mut orphaned = 0;
    for session_id in state.sessions.keys() {
        if !in_use.contains(session_id.as_str()) {
            orphaned += 1;
            let _ = tokio::process::Command::new("tmux")
                .args(["kill-session", "-t", session_id])
                .output()
                .await;
            let _ = ctx
                .event_tx
                .send(Event::SessionDeleted {
                    id: SessionId::new(session_id),
                })
                .await;
        }
    }

    if orphaned > 0 {
        info!("Reconciled {orphaned} orphaned session(s) from terminal/missing jobs");
    }
}

/// Reconcile persisted state with actual world state after daemon restart.
///
/// For each non-terminal job, checks whether its tmux session and agent
/// process are still alive, then either reconnects monitoring or triggers
/// appropriate exit handling through the event channel.
pub(crate) async fn reconcile_state(ctx: &ReconcileCtx) {
    // Reconcile sessions: clean up orphaned sessions whose jobs are terminal or missing
    reconcile_sessions(ctx).await;

    let state = &ctx.state_snapshot;

    // Resume workers that were running before the daemon restarted.
    // Re-emitting WorkerStarted recreates the in-memory WorkerState and
    // triggers an initial queue poll so the worker picks up where it left off.
    let running_workers: Vec<_> = state
        .workers
        .values()
        .filter(|w| w.status == "running")
        .collect();

    if !running_workers.is_empty() {
        info!("Resuming {} running workers", running_workers.len());
    }

    for worker in &running_workers {
        info!(
            worker = %worker.name,
            namespace = %worker.namespace,
            "resuming worker after daemon restart"
        );
        let _ = ctx
            .event_tx
            .send(Event::WorkerStarted {
                worker_name: worker.name.clone(),
                project_root: worker.project_root.clone(),
                runbook_hash: worker.runbook_hash.clone(),
                queue_name: worker.queue_name.clone(),
                concurrency: worker.concurrency,
                namespace: worker.namespace.clone(),
            })
            .await;
    }

    // Resume crons that were running before the daemon restarted.
    let running_crons: Vec<_> = state
        .crons
        .values()
        .filter(|c| c.status == "running")
        .collect();

    if !running_crons.is_empty() {
        info!("Resuming {} running crons", running_crons.len());
    }

    for cron in &running_crons {
        info!(
            cron = %cron.name,
            namespace = %cron.namespace,
            "resuming cron after daemon restart"
        );
        let _ = ctx
            .event_tx
            .send(Event::CronStarted {
                cron_name: cron.name.clone(),
                project_root: cron.project_root.clone(),
                runbook_hash: cron.runbook_hash.clone(),
                interval: cron.interval.clone(),
                run_target: cron.run_target.clone(),
                namespace: cron.namespace.clone(),
            })
            .await;
    }

    // Reconcile standalone agent runs
    let non_terminal_runs: Vec<_> = state
        .agent_runs
        .values()
        .filter(|ar| !ar.is_terminal())
        .collect();

    if !non_terminal_runs.is_empty() {
        info!(
            "Reconciling {} non-terminal standalone agent runs",
            non_terminal_runs.len()
        );
    }

    for agent_run in &non_terminal_runs {
        let Some(ref session_id) = agent_run.session_id else {
            warn!(agent_run_id = %agent_run.id, "no session_id, marking failed");
            let _ = ctx
                .event_tx
                .send(Event::AgentRunStatusChanged {
                    id: AgentRunId::new(&agent_run.id),
                    status: AgentRunStatus::Failed,
                    reason: Some("no session at recovery".to_string()),
                })
                .await;
            continue;
        };

        // If the agent_run has no agent_id, the agent was never fully spawned
        // (daemon crashed before AgentRunStarted was persisted). Directly mark
        // it failed — we can't route through AgentExited/AgentGone events because
        // the handler verifies agent_id matches.
        let Some(ref agent_id_str) = agent_run.agent_id else {
            warn!(agent_run_id = %agent_run.id, "no agent_id, marking failed");
            let _ = ctx
                .event_tx
                .send(Event::AgentRunStatusChanged {
                    id: AgentRunId::new(&agent_run.id),
                    status: AgentRunStatus::Failed,
                    reason: Some("no agent_id at recovery".to_string()),
                })
                .await;
            continue;
        };

        let is_alive = ctx
            .session_adapter
            .is_alive(session_id)
            .await
            .unwrap_or(false);

        if is_alive {
            let process_name = "claude";
            let is_running = ctx
                .session_adapter
                .is_process_running(session_id, process_name)
                .await
                .unwrap_or(false);

            if is_running {
                info!(
                    agent_run_id = %agent_run.id,
                    session_id,
                    "recovering: standalone agent still running, reconnecting watcher"
                );
                if let Err(e) = ctx.runtime.recover_standalone_agent(agent_run).await {
                    warn!(
                        agent_run_id = %agent_run.id,
                        error = %e,
                        "failed to recover standalone agent, marking failed"
                    );
                    let _ = ctx
                        .event_tx
                        .send(Event::AgentRunStatusChanged {
                            id: AgentRunId::new(&agent_run.id),
                            status: AgentRunStatus::Failed,
                            reason: Some(format!("recovery failed: {}", e)),
                        })
                        .await;
                }
            } else {
                info!(
                    agent_run_id = %agent_run.id,
                    session_id,
                    "recovering: standalone agent exited while daemon was down"
                );
                let agent_id = AgentId::new(agent_id_str);
                let agent_run_id = AgentRunId::new(&agent_run.id);
                ctx.runtime
                    .register_agent(agent_id.clone(), OwnerId::agent_run(agent_run_id.clone()));
                let _ = ctx
                    .event_tx
                    .send(Event::AgentExited {
                        agent_id,
                        exit_code: None,
                        owner: OwnerId::agent_run(agent_run_id),
                    })
                    .await;
            }
        } else {
            info!(
                agent_run_id = %agent_run.id,
                session_id,
                "recovering: standalone agent session died while daemon was down"
            );
            let agent_id = AgentId::new(agent_id_str);
            let agent_run_id = AgentRunId::new(&agent_run.id);
            ctx.runtime
                .register_agent(agent_id.clone(), OwnerId::agent_run(agent_run_id.clone()));
            let _ = ctx
                .event_tx
                .send(Event::AgentGone {
                    agent_id,
                    owner: OwnerId::agent_run(agent_run_id),
                })
                .await;
        }
    }

    // Reconcile jobs
    let non_terminal: Vec<_> = state.jobs.values().filter(|p| !p.is_terminal()).collect();

    if non_terminal.is_empty() {
        return;
    }

    info!("Reconciling {} non-terminal jobs", non_terminal.len());

    for job in &non_terminal {
        // Waiting jobs (escalated to human) still need their watcher reconnected
        // so that agent state changes are detected after decision resolution.
        if job.step_status.is_waiting() {
            if let Some(ref session_id) = job.session_id {
                let is_alive = ctx
                    .session_adapter
                    .is_alive(session_id)
                    .await
                    .unwrap_or(false);
                if is_alive {
                    info!(job_id = %job.id, "reconnecting watcher for Waiting job");
                    if let Err(e) = ctx.runtime.recover_agent(job).await {
                        warn!(
                            job_id = %job.id,
                            error = %e,
                            "failed to reconnect watcher for Waiting job"
                        );
                    }
                }
            }
            continue;
        }

        // Determine the tmux session ID
        let Some(session_id) = &job.session_id else {
            warn!(job_id = %job.id, "no session_id, marking failed");
            let _ = ctx
                .event_tx
                .send(Event::JobAdvanced {
                    id: JobId::new(job.id.clone()),
                    step: "failed".to_string(),
                })
                .await;
            continue;
        };

        // Extract agent_id from step_history (stored when agent was spawned).
        // This must match the UUID used during spawn — using any other format
        // causes the handler's stale-event check to drop the event.
        let agent_id_str = job
            .step_history
            .iter()
            .rfind(|r| r.name == job.step)
            .and_then(|r| r.agent_id.clone());

        // Check tmux session liveness
        let is_alive = ctx
            .session_adapter
            .is_alive(session_id)
            .await
            .unwrap_or(false);

        if is_alive {
            let is_running = ctx
                .session_adapter
                .is_process_running(session_id, "claude")
                .await
                .unwrap_or(false);

            if is_running {
                // Case 1: tmux alive + agent running → reconnect watcher
                info!(
                    job_id = %job.id,
                    session_id,
                    "recovering: agent still running, reconnecting watcher"
                );
                if let Err(e) = ctx.runtime.recover_agent(job).await {
                    warn!(
                        job_id = %job.id,
                        error = %e,
                        "failed to recover agent, triggering exit"
                    );
                    // recover_agent extracts agent_id from step_history internally,
                    // so if it failed, use our extracted agent_id (or a fallback).
                    let aid = agent_id_str
                        .clone()
                        .unwrap_or_else(|| format!("{}-{}", job.id, job.step));
                    let agent_id = AgentId::new(aid);
                    let job_id = JobId::new(job.id.clone());
                    let _ = ctx
                        .event_tx
                        .send(Event::AgentGone {
                            agent_id,
                            owner: OwnerId::job(job_id),
                        })
                        .await;
                }
            } else {
                // Case 2: tmux alive, agent dead → trigger on_dead
                let Some(ref aid) = agent_id_str else {
                    warn!(
                        job_id = %job.id,
                        "no agent_id in step_history, marking failed"
                    );
                    let _ = ctx
                        .event_tx
                        .send(Event::JobAdvanced {
                            id: JobId::new(job.id.clone()),
                            step: "failed".to_string(),
                        })
                        .await;
                    continue;
                };
                info!(
                    job_id = %job.id,
                    session_id,
                    "recovering: agent exited while daemon was down"
                );
                let agent_id = AgentId::new(aid);
                let job_id = JobId::new(job.id.to_string());
                // Register mapping so handle_agent_state_changed can find it
                ctx.runtime
                    .register_agent(agent_id.clone(), OwnerId::job(job_id.clone()));
                let _ = ctx
                    .event_tx
                    .send(Event::AgentExited {
                        agent_id,
                        exit_code: None,
                        owner: OwnerId::job(job_id),
                    })
                    .await;
            }
        } else {
            // Case 3: tmux dead → trigger session gone
            let Some(ref aid) = agent_id_str else {
                warn!(
                    job_id = %job.id,
                    "no agent_id in step_history, marking failed"
                );
                let _ = ctx
                    .event_tx
                    .send(Event::JobAdvanced {
                        id: JobId::new(job.id.clone()),
                        step: "failed".to_string(),
                    })
                    .await;
                continue;
            };
            info!(
                job_id = %job.id,
                session_id,
                "recovering: tmux session died while daemon was down"
            );
            let agent_id = AgentId::new(aid);
            let job_id = JobId::new(job.id.clone());
            ctx.runtime
                .register_agent(agent_id.clone(), OwnerId::job(job_id.clone()));
            let _ = ctx
                .event_tx
                .send(Event::AgentGone {
                    agent_id,
                    owner: OwnerId::job(job_id),
                })
                .await;
        }
    }
}
