// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer event handling

use super::super::Runtime;
use crate::error::RuntimeError;
use crate::monitor::{self, MonitorState};
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{AgentId, AgentState, Clock, Effect, Event, PipelineId, TimerId};
use std::time::Duration;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Route timer events to the appropriate handler
    pub(crate) async fn handle_timer(&self, id: &TimerId) -> Result<Vec<Event>, RuntimeError> {
        let id_str = id.as_str();
        if let Some(pipeline_id) = id_str.strip_prefix("liveness:") {
            return self.handle_liveness_timer(pipeline_id).await;
        }
        if let Some(pipeline_id) = id_str.strip_prefix("exit-deferred:") {
            return self.handle_exit_deferred_timer(pipeline_id).await;
        }
        if let Some(rest) = id_str.strip_prefix("cooldown:") {
            return self.handle_cooldown_timer(rest).await;
        }
        if let Some(rest) = id_str.strip_prefix("queue-retry:") {
            return self.handle_queue_retry_timer(rest).await;
        }
        // Unknown timer — no-op
        tracing::debug!(timer_id = %id, "ignoring unknown timer");
        Ok(vec![])
    }

    /// Handle cooldown timer expiry - re-trigger the action
    async fn handle_cooldown_timer(&self, rest: &str) -> Result<Vec<Event>, RuntimeError> {
        // Parse timer ID: "pipeline_id:trigger:chain_pos"
        let parts: Vec<&str> = rest.splitn(3, ':').collect();
        if parts.len() != 3 {
            tracing::warn!(timer_rest = rest, "malformed cooldown timer ID");
            return Ok(vec![]);
        }
        let pipeline_id = parts[0];
        let trigger = parts[1];
        let chain_pos: usize = parts[2].parse().unwrap_or(0);

        let pipeline = match self.get_pipeline(pipeline_id) {
            Some(p) => p,
            None => {
                tracing::debug!(pipeline_id, "cooldown timer for missing pipeline");
                return Ok(vec![]);
            }
        };

        if pipeline.is_terminal() {
            return Ok(vec![]);
        }

        let runbook = self.cached_runbook(&pipeline.runbook_hash)?;
        let agent_def = match monitor::get_agent_def(&runbook, &pipeline) {
            Ok(def) => def.clone(),
            Err(_) => {
                // Pipeline already advanced past the agent step
                return Ok(vec![]);
            }
        };

        // Get the action config based on trigger type
        let action_config = match trigger {
            "idle" => agent_def.on_idle.clone(),
            "exit" => agent_def.on_dead.clone(),
            _ => {
                // Error triggers use a different path; for now escalate on unknown
                tracing::warn!(trigger, "unknown trigger for cooldown timer");
                return Ok(vec![]);
            }
        };

        tracing::info!(
            pipeline_id = %pipeline.id,
            trigger,
            chain_pos,
            "cooldown expired, executing action"
        );

        self.execute_action_with_attempts(&pipeline, &agent_def, &action_config, trigger, chain_pos)
            .await
    }

    /// Handle queue retry timer expiry — move item back to Pending and wake workers.
    async fn handle_queue_retry_timer(&self, rest: &str) -> Result<Vec<Event>, RuntimeError> {
        // Parse timer ID: "scoped_queue_name:item_id"
        // The scoped_queue_name may contain '/' (namespace/queue), so split from the right
        let (scoped_queue, item_id) = match rest.rsplit_once(':') {
            Some(pair) => pair,
            None => {
                tracing::warn!(timer_rest = rest, "malformed queue-retry timer ID");
                return Ok(vec![]);
            }
        };

        // Extract namespace and queue_name from the scoped key
        let (namespace, queue_name) = match scoped_queue.split_once('/') {
            Some((ns, q)) => (ns.to_string(), q.to_string()),
            None => (String::new(), scoped_queue.to_string()),
        };

        tracing::info!(
            queue = %queue_name,
            item = item_id,
            namespace = %namespace,
            "queue retry timer fired, resurrecting item"
        );

        // Emit QueueItemRetry event
        let mut result_events = self
            .executor
            .execute_all(vec![Effect::Emit {
                event: Event::QueueItemRetry {
                    queue_name: queue_name.clone(),
                    item_id: item_id.to_string(),
                    namespace: namespace.clone(),
                },
            }])
            .await?;

        // Wake workers attached to this queue
        let worker_names: Vec<String> = {
            let workers = self.worker_states.lock();
            workers
                .iter()
                .filter(|(_, state)| state.queue_name == queue_name && state.namespace == namespace)
                .map(|(name, _)| name.clone())
                .collect()
        };

        for worker_name in worker_names {
            // Strip namespace prefix from worker_name for the event
            let bare_name = if namespace.is_empty() {
                worker_name.clone()
            } else {
                worker_name
                    .strip_prefix(&format!("{}/", namespace))
                    .unwrap_or(&worker_name)
                    .to_string()
            };
            result_events.extend(
                self.executor
                    .execute_all(vec![Effect::Emit {
                        event: Event::WorkerWake {
                            worker_name: bare_name,
                            namespace: namespace.clone(),
                        },
                    }])
                    .await?,
            );
        }

        Ok(result_events)
    }

    /// Periodic liveness check (30s). Checks if tmux session + claude process are alive.
    async fn handle_liveness_timer(&self, pipeline_id: &str) -> Result<Vec<Event>, RuntimeError> {
        let pipeline = match self.get_pipeline(pipeline_id) {
            Some(p) => p,
            None => return Ok(vec![]),
        };

        if pipeline.is_terminal() {
            return Ok(vec![]); // No need to reschedule
        }

        let session_id = match &pipeline.session_id {
            Some(id) => id.clone(),
            None => return Ok(vec![]),
        };

        let is_alive = self.executor.check_session_alive(&session_id).await;

        if is_alive {
            // Reschedule liveness timer
            let pid = PipelineId::new(pipeline_id);
            self.executor
                .execute(Effect::SetTimer {
                    id: TimerId::liveness(&pid),
                    duration: crate::spawn::LIVENESS_INTERVAL,
                })
                .await?;
            Ok(vec![])
        } else {
            // Session dead — schedule deferred exit (5s grace period)
            tracing::info!(pipeline_id, "session dead, scheduling deferred exit");
            let pid = PipelineId::new(pipeline_id);
            self.executor
                .execute(Effect::SetTimer {
                    id: TimerId::exit_deferred(&pid),
                    duration: Duration::from_secs(5),
                })
                .await?;
            Ok(vec![])
        }
    }

    /// Deferred exit handler (5s after liveness detected death).
    /// Reads final session log to determine exit reason.
    async fn handle_exit_deferred_timer(
        &self,
        pipeline_id: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let pipeline = match self.get_pipeline(pipeline_id) {
            Some(p) => p,
            None => return Ok(vec![]),
        };

        // If pipeline already terminal, agent state event won the race — no-op
        if pipeline.is_terminal() {
            return Ok(vec![]);
        }

        // Read final agent state from session log
        // Get agent_id from step history (it's a UUID stored when the agent was spawned)
        let agent_id = pipeline
            .step_history
            .iter()
            .rfind(|r| r.name == pipeline.step)
            .and_then(|r| r.agent_id.clone())
            .map(AgentId::new);

        let final_state = match agent_id {
            Some(id) => self.executor.get_agent_state(&id).await.ok(),
            None => {
                // No agent_id means we can't check state, treat as unexpected death
                tracing::warn!(
                    pipeline_id = %pipeline.id,
                    step = %pipeline.step,
                    "no agent_id in step history for exit deferred timer"
                );
                None
            }
        };

        let runbook = self.cached_runbook(&pipeline.runbook_hash)?;
        let agent_def = match monitor::get_agent_def(&runbook, &pipeline) {
            Ok(def) => def.clone(),
            Err(_) => {
                // Pipeline already advanced past the agent step
                return Ok(vec![]);
            }
        };

        // Map final state to monitor action
        let monitor_state = match final_state {
            Some(AgentState::WaitingForInput) => MonitorState::WaitingForInput,
            Some(AgentState::Failed(err)) => {
                MonitorState::from_agent_state(&AgentState::Failed(err))
            }
            _ => MonitorState::Exited, // on_dead (unexpected death)
        };

        self.handle_monitor_state(&pipeline, &agent_def, monitor_state)
            .await
    }
}
