// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event handling for the runtime

mod agent;
mod command;
mod lifecycle;
mod pipeline_create;
mod timer;
pub(crate) mod worker;

pub(crate) use pipeline_create::CreatePipelineParams;

use super::Runtime;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event};

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
                pipeline_id,
                pipeline_name,
                project_root,
                invoke_dir,
                namespace,
                command,
                args,
            } => {
                result_events.extend(
                    self.handle_command(
                        pipeline_id,
                        pipeline_name,
                        project_root,
                        invoke_dir,
                        namespace,
                        command,
                        args,
                    )
                    .await?,
                );
            }

            Event::AgentWorking { .. }
            | Event::AgentWaiting { .. }
            | Event::AgentFailed { .. }
            | Event::AgentExited { .. }
            | Event::AgentGone { .. } => {
                if let Some((agent_id, state)) = event.as_agent_state() {
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
                // Just handle pipeline advance
                result_events.extend(
                    self.handle_agent_done(agent_id, kind.clone(), message.clone())
                        .await?,
                );
            }

            Event::AgentIdle { agent_id } => {
                result_events.extend(self.handle_agent_idle_hook(agent_id).await?);
            }

            Event::AgentPrompt {
                agent_id,
                prompt_type,
            } => {
                result_events.extend(self.handle_agent_prompt_hook(agent_id, prompt_type).await?);
            }

            Event::ShellExited {
                pipeline_id,
                step,
                exit_code,
            } => {
                result_events.extend(
                    self.handle_shell_exited(pipeline_id, step, *exit_code)
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

            Event::PipelineResume { id, message, vars } => {
                result_events.extend(
                    self.handle_pipeline_resume(id, message.as_deref(), vars)
                        .await?,
                );
            }

            Event::PipelineCancel { id } => {
                result_events.extend(self.handle_pipeline_cancel(id).await?);
            }

            Event::WorkspaceDrop { id } => {
                result_events.extend(self.handle_workspace_drop(id).await?);
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

            Event::WorkerStopped { worker_name, .. } => {
                result_events.extend(self.handle_worker_stopped(worker_name).await?);
            }

            // Pipeline terminal state -> check worker re-poll
            Event::PipelineAdvanced { id, step }
                if step == "done" || step == "failed" || step == "cancelled" =>
            {
                result_events.extend(self.check_worker_pipeline_complete(id, step).await?);
            }

            // Queue pushed -> wake workers watching this queue
            Event::QueuePushed {
                queue_name,
                namespace,
                ..
            } => {
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
            Event::QueueTaken { .. }
            | Event::QueueCompleted { .. }
            | Event::QueueFailed { .. }
            | Event::QueueDropped { .. } => {}

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

            // No-op: signals and state mutations handled elsewhere
            Event::Shutdown
            | Event::Custom
            | Event::PipelineCreated { .. }
            | Event::PipelineAdvanced { .. }
            | Event::StepStarted { .. }
            | Event::StepWaiting { .. }
            | Event::StepCompleted { .. }
            | Event::StepFailed { .. }
            | Event::PipelineDeleted { .. }
            | Event::SessionCreated { .. }
            | Event::SessionDeleted { .. }
            | Event::WorkspaceCreated { .. }
            | Event::WorkspaceReady { .. }
            | Event::WorkspaceFailed { .. }
            | Event::WorkspaceDeleted { .. }
            | Event::WorkerDeleted { .. }
            | Event::PipelineCancelling { .. }
            | Event::PipelineUpdated { .. }
            | Event::WorkerItemDispatched { .. }
            | Event::QueueItemRetry { .. }
            | Event::QueueItemDead { .. } => {}
        }

        Ok(result_events)
    }
}
