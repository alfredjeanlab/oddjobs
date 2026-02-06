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
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{scoped_name, Clock, Effect, Event};

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Handle an incoming event and return any produced events
    pub async fn handle_event(&self, event: Event) -> Result<Vec<Event>, RuntimeError> {
        let mut result_events = Vec::new();

        match &event {
            Event::CommandRun {
                job_id,
                job_name,
                project_root,
                invoke_dir,
                namespace,
                command,
                args,
            } => {
                result_events.extend(
                    self.handle_command(HandleCommandParams {
                        job_id,
                        job_name,
                        project_root,
                        invoke_dir,
                        namespace,
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
                    result_events.extend(self.handle_agent_state_changed(agent_id, &state).await?);
                }
            }

            Event::AgentInput { agent_id, input } => {
                self.executor
                    .execute(Effect::SendToAgent {
                        agent_id: agent_id.clone(),
                        input: input.clone(),
                    })
                    .await?;
            }

            Event::AgentSignal {
                agent_id,
                kind,
                message,
            } => {
                // Signal is now persisted via WAL in MaterializedState
                // Just handle job advance
                result_events.extend(
                    self.handle_agent_done(agent_id, kind.clone(), message.clone())
                        .await?,
                );
            }

            Event::AgentIdle { agent_id } => {
                result_events.extend(self.handle_agent_idle_hook(agent_id).await?);
            }

            Event::AgentStop { agent_id } => {
                result_events.extend(self.handle_agent_stop_hook(agent_id).await?);
            }

            Event::AgentPrompt {
                agent_id,
                prompt_type,
                question_data,
            } => {
                result_events.extend(
                    self.handle_agent_prompt_hook(agent_id, prompt_type, question_data.as_ref())
                        .await?,
                );
            }

            Event::ShellExited {
                job_id,
                step,
                exit_code,
                stdout,
                stderr,
            } => {
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

            Event::SessionInput { id, input } => {
                self.executor
                    .execute(Effect::SendToSession {
                        session_id: id.clone(),
                        input: format!("{}\n", input),
                    })
                    .await?;
            }

            Event::JobResume {
                id,
                message,
                vars,
                kill,
            } => {
                result_events.extend(
                    self.handle_job_resume(id, message.as_deref(), vars, *kill)
                        .await?,
                );
            }

            Event::JobCancel { id } => {
                result_events.extend(self.handle_job_cancel(id).await?);
            }

            Event::WorkspaceDrop { id } => {
                result_events.extend(self.handle_workspace_drop(id).await?);
            }

            // -- cron events --
            Event::CronStarted {
                cron_name,
                project_root,
                runbook_hash,
                interval,
                run_target,
                namespace,
            } => {
                result_events.extend(
                    self.handle_cron_started(CronStartedParams {
                        cron_name,
                        project_root,
                        runbook_hash,
                        interval,
                        run_target,
                        namespace,
                    })
                    .await?,
                );
            }

            Event::CronStopped {
                cron_name,
                namespace,
            } => {
                result_events.extend(self.handle_cron_stopped(cron_name, namespace).await?);
            }

            Event::CronOnce {
                cron_name,
                job_id,
                job_name,
                job_kind,
                agent_run_id,
                agent_name,
                project_root,
                runbook_hash,
                run_target,
                namespace,
            } => {
                result_events.extend(
                    self.handle_cron_once(CronOnceParams {
                        cron_name,
                        job_id,
                        job_name,
                        job_kind,
                        agent_run_id,
                        agent_name,
                        runbook_hash,
                        run_target,
                        namespace,
                        project_root,
                    })
                    .await?,
                );
            }

            // -- worker events --
            Event::WorkerStarted {
                worker_name,
                project_root,
                runbook_hash,
                namespace,
                ..
            } => {
                result_events.extend(
                    self.handle_worker_started(worker_name, project_root, runbook_hash, namespace)
                        .await?,
                );
            }

            Event::WorkerWake { worker_name, .. } => {
                result_events.extend(self.handle_worker_wake(worker_name).await?);
            }

            Event::WorkerPollComplete {
                worker_name, items, ..
            } => {
                result_events.extend(self.handle_worker_poll_complete(worker_name, items).await?);
            }

            Event::WorkerTakeComplete {
                worker_name,
                item_id,
                item,
                exit_code,
                stderr,
            } => {
                result_events.extend(
                    self.handle_worker_take_complete(
                        worker_name,
                        item_id,
                        item,
                        *exit_code,
                        stderr.as_deref(),
                    )
                    .await?,
                );
            }

            Event::WorkerStopped { worker_name, .. } => {
                result_events.extend(self.handle_worker_stopped(worker_name).await?);
            }

            Event::WorkerResized {
                worker_name,
                concurrency,
                namespace,
            } => {
                result_events.extend(
                    self.handle_worker_resized(worker_name, *concurrency, namespace)
                        .await?,
                );
            }

            // Job terminal state -> check worker re-poll
            // NOTE: check_worker_job_complete is also called directly from
            // fail_job/cancel_job/complete_job for immediate queue
            // item updates. This handler is a no-op safety net (idempotent).
            Event::JobAdvanced { id, step }
                if step == "done" || step == "failed" || step == "cancelled" =>
            {
                result_events.extend(self.check_worker_job_complete(id, step).await?);
            }

            // Queue pushed -> wake workers watching this queue
            Event::QueuePushed {
                queue_name,
                namespace,
                item_id,
                data,
                ..
            } => {
                // Log queue push event
                let scoped = scoped_name(namespace, queue_name);
                let data_str = data
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.queue_logger.append(
                    &scoped,
                    item_id,
                    &format!("pushed data={{{}}}", data_str),
                );

                let (worker_names, all_workers): (Vec<String>, Vec<String>) = {
                    let workers = self.worker_states.lock();
                    let all: Vec<String> = workers.keys().cloned().collect();
                    let matching: Vec<String> = workers
                        .iter()
                        .filter(|(_, state)| {
                            state.queue_name == *queue_name
                                && state.status == worker::WorkerStatus::Running
                        })
                        .map(|(name, _)| name.clone())
                        .collect();
                    (matching, all)
                };

                tracing::info!(
                    queue = queue_name.as_str(),
                    matched = ?worker_names,
                    registered = ?all_workers,
                    "queue pushed: waking workers"
                );

                for name in worker_names {
                    result_events.extend(
                        self.executor
                            .execute_all(vec![Effect::Emit {
                                event: Event::WorkerWake {
                                    worker_name: name,
                                    namespace: namespace.clone(),
                                },
                            }])
                            .await?,
                    );
                }
            }

            // Queue state mutations handled by MaterializedState::apply_event
            // Log queue lifecycle events
            Event::QueueTaken {
                queue_name,
                item_id,
                worker_name,
                namespace,
            } => {
                let scoped = scoped_name(namespace, queue_name);
                self.queue_logger.append(
                    &scoped,
                    item_id,
                    &format!("dispatched worker={}", worker_name),
                );
            }
            Event::QueueCompleted {
                queue_name,
                item_id,
                namespace,
            } => {
                let scoped = scoped_name(namespace, queue_name);
                self.queue_logger.append(&scoped, item_id, "completed");
            }
            Event::QueueFailed {
                queue_name,
                item_id,
                error,
                namespace,
            } => {
                let scoped = scoped_name(namespace, queue_name);
                self.queue_logger
                    .append(&scoped, item_id, &format!("failed error=\"{}\"", error));
            }
            Event::QueueDropped {
                queue_name,
                item_id,
                namespace,
            } => {
                let scoped = scoped_name(namespace, queue_name);
                self.queue_logger.append(&scoped, item_id, "dropped");
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

            // No-op: signals and state mutations handled elsewhere
            Event::Shutdown
            | Event::Custom
            | Event::JobCreated { .. }
            | Event::JobAdvanced { .. }
            | Event::StepStarted { .. }
            | Event::StepWaiting { .. }
            | Event::StepCompleted { .. }
            | Event::StepFailed { .. }
            | Event::SessionCreated { .. }
            | Event::SessionDeleted { .. }
            | Event::WorkspaceCreated { .. }
            | Event::WorkspaceReady { .. }
            | Event::WorkspaceFailed { .. }
            | Event::WorkspaceDeleted { .. }
            | Event::WorkerDeleted { .. }
            | Event::JobCancelling { .. }
            | Event::JobUpdated { .. }
            | Event::WorkerItemDispatched { .. }
            | Event::CronFired { .. }
            | Event::CronDeleted { .. }
            | Event::DecisionCreated { .. }
            | Event::DecisionResolved { .. }
            | Event::AgentRunCreated { .. }
            | Event::AgentRunStarted { .. }
            | Event::AgentRunStatusChanged { .. }
            | Event::AgentRunDeleted { .. } => {}

            Event::AgentRunResume { id, message, kill } => {
                result_events.extend(
                    self.handle_agent_run_resume(id, message.as_deref(), *kill)
                        .await?,
                );
            }

            // Queue retry/dead: log lifecycle events
            Event::QueueItemRetry {
                queue_name,
                item_id,
                namespace,
            } => {
                let scoped = scoped_name(namespace, queue_name);
                self.queue_logger.append(&scoped, item_id, "retried");
            }
            Event::QueueItemDead {
                queue_name,
                item_id,
                namespace,
            } => {
                let scoped = scoped_name(namespace, queue_name);
                self.queue_logger.append(&scoped, item_id, "dead");
            }
        }

        Ok(result_events)
    }
}
