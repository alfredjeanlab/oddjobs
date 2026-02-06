// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! State reconciliation after daemon restart.
//!
//! Checks persisted state against actual tmux sessions and reconnects
//! monitoring or triggers appropriate exit handling for each entity.

use oj_adapters::{SessionAdapter, TmuxAdapter, TracedSession};
use oj_core::{AgentId, AgentRunId, AgentRunStatus, Event, JobId, OwnerId, SessionId};
use oj_storage::MaterializedState;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::DaemonRuntime;

/// Reconcile sessions with actual tmux state after daemon restart.
///
/// Cleans up orphaned sessions whose jobs are terminal or missing from state.
/// This prevents stale sessions from accumulating across daemon restarts.
async fn reconcile_sessions(
    state: &MaterializedState,
    _sessions: &TracedSession<TmuxAdapter>,
    event_tx: &mpsc::Sender<Event>,
) {
    let sessions_to_check: Vec<_> = state
        .sessions
        .values()
        .map(|s| (s.id.clone(), s.job_id.clone()))
        .collect();

    if sessions_to_check.is_empty() {
        return;
    }

    let mut orphaned = 0;
    for (session_id, job_id) in sessions_to_check {
        // Check if the associated job is terminal or missing
        let should_prune = match state.jobs.get(&job_id) {
            Some(job) => job.is_terminal(),
            None => {
                // Job doesn't exist - check if it's a standalone agent run
                // Sessions can also be associated with agent_runs
                let has_agent_run = state.agent_runs.values().any(|ar| {
                    ar.session_id.as_deref() == Some(session_id.as_str()) && !ar.is_terminal()
                });
                !has_agent_run
            }
        };

        if should_prune {
            orphaned += 1;

            // Kill the tmux session (best effort)
            let _ = tokio::process::Command::new("tmux")
                .args(["kill-session", "-t", &session_id])
                .output()
                .await;

            // Emit SessionDeleted to clean up state
            let _ = event_tx
                .send(Event::SessionDeleted {
                    id: SessionId::new(&session_id),
                })
                .await;
        }
    }

    if orphaned > 0 {
        info!(
            "Reconciled {} orphaned session(s) from terminal/missing jobs",
            orphaned
        );
    }
}

/// Reconcile persisted state with actual world state after daemon restart.
///
/// For each non-terminal job, checks whether its tmux session and agent
/// process are still alive, then either reconnects monitoring or triggers
/// appropriate exit handling through the event channel.
pub(crate) async fn reconcile_state(
    runtime: &DaemonRuntime,
    state: &MaterializedState,
    sessions: &TracedSession<TmuxAdapter>,
    event_tx: &mpsc::Sender<Event>,
) {
    // Reconcile sessions: clean up orphaned sessions whose jobs are terminal or missing
    reconcile_sessions(state, sessions, event_tx).await;

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
        let _ = event_tx
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
        let _ = event_tx
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
            let _ = event_tx
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
            let _ = event_tx
                .send(Event::AgentRunStatusChanged {
                    id: AgentRunId::new(&agent_run.id),
                    status: AgentRunStatus::Failed,
                    reason: Some("no agent_id at recovery".to_string()),
                })
                .await;
            continue;
        };

        let is_alive = sessions.is_alive(session_id).await.unwrap_or(false);

        if is_alive {
            let process_name = "claude";
            let is_running = sessions
                .is_process_running(session_id, process_name)
                .await
                .unwrap_or(false);

            if is_running {
                info!(
                    agent_run_id = %agent_run.id,
                    session_id,
                    "recovering: standalone agent still running, reconnecting watcher"
                );
                if let Err(e) = runtime.recover_standalone_agent(agent_run).await {
                    warn!(
                        agent_run_id = %agent_run.id,
                        error = %e,
                        "failed to recover standalone agent, marking failed"
                    );
                    let _ = event_tx
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
                runtime.register_agent(agent_id.clone(), OwnerId::agent_run(agent_run_id.clone()));
                let _ = event_tx
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
            runtime.register_agent(agent_id.clone(), OwnerId::agent_run(agent_run_id.clone()));
            let _ = event_tx
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
        // Skip jobs in Waiting status — already escalated to human
        if job.step_status.is_waiting() {
            info!(
                job_id = %job.id,
                "skipping Waiting job (already escalated)"
            );
            continue;
        }

        // Determine the tmux session ID
        let Some(session_id) = &job.session_id else {
            warn!(job_id = %job.id, "no session_id, skipping");
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
        let is_alive = sessions.is_alive(session_id).await.unwrap_or(false);

        if is_alive {
            let is_running = sessions
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
                if let Err(e) = runtime.recover_agent(job).await {
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
                    let _ = event_tx
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
                        "no agent_id in step_history, cannot route exit event"
                    );
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
                runtime.register_agent(agent_id.clone(), OwnerId::job(job_id.clone()));
                let _ = event_tx
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
                    "no agent_id in step_history, cannot route gone event"
                );
                continue;
            };
            info!(
                job_id = %job.id,
                session_id,
                "recovering: tmux session died while daemon was down"
            );
            let agent_id = AgentId::new(aid);
            let job_id = JobId::new(job.id.clone());
            runtime.register_agent(agent_id.clone(), OwnerId::job(job_id.clone()));
            let _ = event_tx
                .send(Event::AgentGone {
                    agent_id,
                    owner: OwnerId::job(job_id),
                })
                .await;
        }
    }
}
