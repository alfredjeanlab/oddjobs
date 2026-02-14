// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event handling for the runtime

mod agent;
mod command;
pub(crate) mod cron;
mod job_create;
mod lifecycle;
mod timer;
pub(crate) mod worker;

pub(crate) use job_create::CreateJobParams;

use self::command::HandleCommandParams;
use self::cron::{CronOnceParams, CronStartedParams};
use super::Runtime;
use crate::engine::error::RuntimeError;
use oj_core::{scoped_name, split_scoped_name, Clock, Effect, Event};

impl<C: Clock> Runtime<C> {
    /// Handle an incoming event and return any produced events
    pub async fn handle_event(&self, event: Event) -> Result<Vec<Event>, RuntimeError> {
        let mut result_events = Vec::new();

        match &event {
            Event::CommandRun { owner, name, project_path, invoke_dir, project, command, args } => {
                result_events.extend(
                    self.handle_command(HandleCommandParams {
                        owner,
                        name,
                        project_path,
                        invoke_dir,
                        project,
                        command,
                        args,
                    })
                    .await?,
                );
            }

            Event::AgentWorking { .. }
            | Event::AgentWaiting { .. }
            | Event::AgentFailed { .. }
            | Event::AgentExited { .. }
            | Event::AgentGone { .. } => {
                // Note: owner is used for WAL replay routing in state.rs, not here.
                // The runtime routes by agent_id through its agent_owners map.
                if let Some((agent_id, state, _owner)) = event.as_agent_state() {
                    result_events.extend(self.handle_agent_status(agent_id, &state).await?);
                }
            }

            Event::AgentInput { id: agent_id, input } => {
                self.executor
                    .execute(Effect::SendToAgent { agent_id: *agent_id, input: input.clone() })
                    .await?;
            }

            Event::AgentRespond { id: agent_id, response } => {
                self.executor
                    .execute(Effect::RespondToAgent {
                        agent_id: *agent_id,
                        response: response.clone(),
                    })
                    .await?;
            }

            Event::AgentIdle { id: agent_id } => {
                result_events.extend(self.handle_agent_idle_hook(agent_id).await?);
            }

            Event::AgentStopBlocked { id: agent_id } => {
                result_events.extend(self.handle_agent_stop_blocked(agent_id).await?);
            }

            Event::AgentStopAllowed { id: agent_id } => {
                result_events.extend(self.handle_agent_stop_allowed(agent_id).await?);
            }

            Event::AgentPrompt { id: agent_id, prompt_type, questions, last_message } => {
                result_events.extend(
                    self.handle_agent_prompt_hook(
                        agent_id,
                        prompt_type,
                        questions.as_ref(),
                        last_message.as_deref(),
                    )
                    .await?,
                );
            }

            Event::ShellExited { job_id, step, exit_code, stdout, stderr } => {
                result_events.extend(
                    self.handle_shell_exited(
                        job_id,
                        step,
                        *exit_code,
                        stdout.as_deref(),
                        stderr.as_deref(),
                    )
                    .await?,
                );
            }

            Event::TimerStart { id } => {
                result_events.extend(self.handle_timer(id).await?);
            }

            Event::JobResume { id, message, vars, kill } => {
                result_events
                    .extend(self.handle_job_resume(id, message.as_deref(), vars, *kill).await?);
            }

            Event::JobCancel { id } => {
                result_events.extend(self.handle_job_cancel(id).await?);
            }

            Event::JobSuspend { id } => {
                result_events.extend(self.handle_job_suspend(id).await?);
            }

            Event::WorkspaceDrop { id } => {
                result_events.extend(self.handle_workspace_drop(id).await?);
            }

            // -- cron events --
            Event::CronStarted { cron, project_path, runbook_hash, interval, target, project } => {
                result_events.extend(
                    self.handle_cron_started(CronStartedParams {
                        cron,
                        project_path,
                        runbook_hash,
                        interval,
                        target,
                        project,
                    })
                    .await?,
                );
            }

            Event::CronStopped { cron, project } => {
                result_events.extend(self.handle_cron_stopped(cron, project).await?);
            }

            Event::CronOnce { cron, owner, project, project_path, runbook_hash, target } => {
                result_events.extend(
                    self.handle_cron_once(CronOnceParams {
                        cron,
                        owner,
                        runbook_hash,
                        target,
                        project,
                        project_path,
                    })
                    .await?,
                );
            }

            // -- worker events --
            Event::WorkerStarted { worker, project, project_path, runbook_hash, .. } => {
                result_events.extend(
                    self.handle_worker_started(worker, project_path, runbook_hash, project).await?,
                );
            }

            Event::WorkerWake { worker, project } => {
                let worker_key = scoped_name(project, worker);
                result_events.extend(self.handle_worker_wake(&worker_key).await?);
            }

            Event::WorkerPolled { worker, project, items, .. } => {
                let worker_key = scoped_name(project, worker);
                result_events.extend(self.handle_worker_poll_complete(&worker_key, items).await?);
            }

            Event::WorkerTook { worker, project, item_id, item, exit_code, stderr } => {
                let worker_key = scoped_name(project, worker);
                result_events.extend(
                    self.handle_worker_take_complete(
                        &worker_key,
                        item_id,
                        item,
                        *exit_code,
                        stderr.as_deref(),
                    )
                    .await?,
                );
            }

            Event::WorkerStopped { worker, project } => {
                let worker_key = scoped_name(project, worker);
                result_events.extend(self.handle_worker_stopped(&worker_key).await?);
            }

            Event::WorkerResized { worker, project, concurrency } => {
                result_events
                    .extend(self.handle_worker_resized(worker, *concurrency, project).await?);
            }

            // Job terminal state -> check worker re-poll
            // NOTE: check_worker_job_complete is also called directly from
            // fail_job/cancel_job/complete_job for immediate queue
            // item updates. This handler is a no-op safety net (idempotent).
            Event::JobAdvanced { id, step }
                if step == "done"
                    || step == "failed"
                    || step == "cancelled"
                    || step == "suspended" =>
            {
                result_events.extend(self.check_worker_job_complete(id, step).await?);
            }

            // Queue pushed -> wake workers watching this queue
            Event::QueuePushed { queue, project, item_id, data, .. } => {
                // Log queue push event
                let scoped = scoped_name(project, queue);
                let data_str =
                    data.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(", ");
                self.queue_logger.append(
                    &scoped,
                    item_id,
                    &format!("pushed data={{{}}}", data_str),
                );

                let (worker_keys, all_workers): (Vec<String>, Vec<String>) = {
                    let workers = self.worker_states.lock();
                    let all: Vec<String> = workers.keys().cloned().collect();
                    let matching: Vec<String> = workers
                        .iter()
                        .filter(|(_, state)| {
                            state.queue_name == *queue
                                && state.project == *project
                                && state.status == worker::WorkerStatus::Running
                        })
                        .map(|(name, _)| name.clone())
                        .collect();
                    (matching, all)
                };

                tracing::info!(
                    queue = queue.as_str(),
                    matched = ?worker_keys,
                    registered = ?all_workers,
                    "queue pushed: waking workers"
                );

                for key in worker_keys {
                    let (_, bare_name) = split_scoped_name(&key);
                    result_events.extend(
                        self.executor
                            .execute_all(vec![Effect::Emit {
                                event: Event::WorkerWake {
                                    worker: bare_name.to_string(),
                                    project: project.clone(),
                                },
                            }])
                            .await?,
                    );
                }
            }

            // Queue state mutations handled by MaterializedState::apply_event
            // Log queue lifecycle events
            Event::QueueTaken { queue, item_id, worker, project } => {
                let scoped = scoped_name(project, queue);
                self.queue_logger.append(
                    &scoped,
                    item_id,
                    &format!("dispatched worker={}", worker),
                );
            }
            Event::QueueCompleted { queue, item_id, project } => {
                let scoped = scoped_name(project, queue);
                self.queue_logger.append(&scoped, item_id, "completed");
            }
            Event::QueueFailed { queue, item_id, error, project } => {
                let scoped = scoped_name(project, queue);
                self.queue_logger.append(&scoped, item_id, &format!("failed error=\"{}\"", error));
            }
            Event::QueueDropped { queue, item_id, project } => {
                let scoped = scoped_name(project, queue);
                self.queue_logger.append(&scoped, item_id, "dropped");
            }
            Event::QueueRetry { queue, item_id, project } => {
                let scoped = scoped_name(project, queue);
                self.queue_logger.append(&scoped, item_id, "retried");
            }
            Event::QueueDead { queue, item_id, project } => {
                let scoped = scoped_name(project, queue);
                self.queue_logger.append(&scoped, item_id, "dead");
            }

            // Populate in-process runbook cache so subsequent WorkerStarted
            // events (including WAL replay after restart) can find the runbook.
            Event::RunbookLoaded { hash, runbook, .. } => {
                let mut cache = self.runbook_cache.lock();
                if !cache.contains_key(hash) {
                    if let Ok(rb) = serde_json::from_value(runbook.clone()) {
                        cache.insert(hash.clone(), rb);
                    }
                }
            }

            Event::JobDeleted { id } => {
                result_events.extend(self.handle_job_deleted(id).await?);
            }

            Event::WorkspaceReady { id } => {
                result_events.extend(self.handle_workspace_ready(id).await?);
            }

            Event::WorkspaceFailed { id, reason } => {
                result_events.extend(self.handle_workspace_failed(id, reason).await?);
            }

            Event::AgentSpawned { owner, .. } => {
                result_events.extend(self.handle_agent_spawned(owner).await?);
            }

            Event::AgentSpawnFailed { id: agent_id, owner, reason } => {
                result_events
                    .extend(self.handle_agent_spawn_failed(agent_id, owner, reason).await?);
            }

            Event::JobCreated { id, .. } => {
                result_events.extend(self.handle_job_created(id).await?);
            }

            Event::CrewResume { id, message, kill } => {
                result_events.extend(self.handle_crew_resume(id, message.as_deref(), *kill).await?);
            }

            // No-op: signals and state mutations handled elsewhere
            Event::Shutdown
            | Event::Custom
            | Event::JobAdvanced { .. }
            | Event::StepStarted { .. }
            | Event::StepWaiting { .. }
            | Event::StepCompleted { .. }
            | Event::StepFailed { .. }
            | Event::WorkspaceCreated { .. }
            | Event::WorkspaceDeleted { .. }
            | Event::WorkerDeleted { .. }
            | Event::JobFailing { .. }
            | Event::JobCancelling { .. }
            | Event::JobSuspending { .. }
            | Event::JobUpdated { .. }
            | Event::WorkerDispatched { .. }
            | Event::CronFired { .. }
            | Event::CronDeleted { .. }
            | Event::DecisionCreated { .. }
            | Event::DecisionResolved { .. }
            | Event::CrewCreated { .. }
            | Event::CrewStarted { .. }
            | Event::CrewUpdated { .. }
            | Event::CrewDeleted { .. } => {}
        }

        Ok(result_events)
    }
}
