// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer event handling

use super::super::Runtime;
use crate::engine::error::RuntimeError;
use crate::engine::monitor::MonitorState;
use crate::engine::ActionContext;
use oj_core::{
    split_scoped_name, AgentId, AgentState, Clock, Effect, Event, OwnerId, TimerId, TimerKind,
};
use std::time::Duration;

impl<C: Clock> Runtime<C> {
    /// Route timer events to the appropriate handler
    pub(crate) async fn handle_timer(&self, id: &TimerId) -> Result<Vec<Event>, RuntimeError> {
        match TimerKind::parse(id.as_str()) {
            Some(TimerKind::Liveness(owner)) => self.handle_owner_liveness(owner).await,
            Some(TimerKind::ExitDeferred(owner)) => self.handle_owner_exit_deferred(owner).await,
            Some(TimerKind::Cooldown { owner, trigger, chain_pos }) => {
                self.handle_owner_cooldown(owner, trigger, chain_pos).await
            }
            Some(TimerKind::QueueRetry { scoped_queue, item_id }) => {
                self.handle_queue_retry_timer(scoped_queue, item_id).await
            }
            Some(TimerKind::Cron { scoped_name }) => {
                self.handle_cron_timer_fired(scoped_name).await
            }
            Some(TimerKind::QueuePoll { scoped_name }) => {
                self.handle_queue_poll_timer(scoped_name).await
            }
            None => Ok(vec![]),
        }
    }

    /// Periodic liveness check. Checks if the agent is alive.
    async fn handle_owner_liveness(&self, owner: OwnerId) -> Result<Vec<Event>, RuntimeError> {
        let agent_id = match self.get_owner_active_agent(&owner) {
            Some(id) => id,
            None => return Ok(vec![]),
        };

        let is_running = self.executor.agents.is_alive(&agent_id).await;

        if is_running {
            self.executor
                .execute(Effect::SetTimer {
                    id: TimerId::liveness(owner),
                    duration: crate::engine::spawn::LIVENESS_INTERVAL,
                })
                .await?;
        } else {
            tracing::info!(%owner, "agent process dead, scheduling deferred exit");
            self.executor
                .execute(Effect::SetTimer {
                    id: TimerId::exit_deferred(owner),
                    duration: Duration::from_secs(5),
                })
                .await?;
        }
        Ok(vec![])
    }

    /// Deferred exit handler (5s after liveness detected death).
    /// Reads final session log to determine exit reason.
    async fn handle_owner_exit_deferred(&self, owner: OwnerId) -> Result<Vec<Event>, RuntimeError> {
        let Some(run) = self.get_active_run(&owner) else {
            return Ok(vec![]);
        };
        let agent_id = run.agent_id().map(AgentId::from_string);

        if agent_id.is_none() {
            tracing::warn!(run_id = %run.log_id(), "no agent_id for exit deferred timer");
        }

        let final_state = match agent_id {
            Some(ref id) => self.executor.get_agent_state(id).await.ok(),
            None => None,
        };

        let monitor_state = match final_state {
            Some(AgentState::WaitingForInput) => MonitorState::WaitingForInput,
            Some(AgentState::Failed(err)) => {
                MonitorState::from_agent_state(&AgentState::Failed(err))
            }
            _ => MonitorState::Exited { exit_code: None },
        };

        let runbook = self.cached_runbook(run.runbook_hash())?;
        let Ok(agent_def) = run.resolve_agent_def(&runbook) else {
            return Ok(vec![]);
        };
        self.handle_monitor_state_for(run.as_ref(), &agent_def, monitor_state).await
    }

    /// Cooldown timer handler — re-trigger the action after cooldown expires.
    async fn handle_owner_cooldown(
        &self,
        owner: OwnerId,
        trigger: &str,
        chain_pos: usize,
    ) -> Result<Vec<Event>, RuntimeError> {
        let Some(run) = self.get_active_run(&owner) else {
            return Ok(vec![]);
        };

        let runbook = self.cached_runbook(run.runbook_hash())?;
        let Ok(agent_def) = run.resolve_agent_def(&runbook) else {
            return Ok(vec![]);
        };

        let action_config = match trigger {
            "idle" => agent_def.on_idle.clone().unwrap_or_default(),
            "exit" => agent_def.on_dead.clone(),
            _ => {
                tracing::warn!(trigger, "unknown trigger for cooldown timer");
                return Ok(vec![]);
            }
        };

        tracing::info!(%owner, trigger, chain_pos, "cooldown expired, executing action");

        let agent_id = run.agent_id().map(AgentId::from_string);
        let last_message = match agent_id {
            Some(aid) => self.executor.agents.last_message(&aid).await,
            None => None,
        };

        let ctx = ActionContext {
            agent_def: &agent_def,
            action_config: &action_config,
            trigger,
            chain_pos,
            questions: None,
            last_message: last_message.as_deref(),
        };

        self.execute_action_with_attempts_for(run.as_ref(), &ctx).await
    }

    /// Get agent_id for a non-terminal owner. Returns None if owner is missing,
    /// terminal, or has no agent.
    fn get_owner_active_agent(&self, owner: &OwnerId) -> Option<AgentId> {
        self.get_active_run(owner)?.agent_id().map(AgentId::from_string)
    }

    /// Handle queue retry timer expiry — move item back to Pending and wake workers.
    async fn handle_queue_retry_timer(
        &self,
        scoped_queue: &str,
        item_id: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let (ns, qn) = split_scoped_name(scoped_queue);
        let (project, queue_name) = (ns.to_string(), qn.to_string());

        tracing::info!(
            queue = %queue_name,
            item = item_id,
            project = %project,
            "queue retry timer fired, resurrecting item"
        );

        let mut result_events = self
            .executor
            .execute_all(vec![Effect::Emit {
                event: Event::QueueRetry {
                    queue: queue_name.clone(),
                    item_id: item_id.to_string(),
                    project: project.clone(),
                },
            }])
            .await?;

        // Wake workers attached to this queue
        let worker_names: Vec<String> = {
            let workers = self.worker_states.lock();
            workers
                .iter()
                .filter(|(_, state)| state.queue_name == queue_name && state.project == project)
                .map(|(name, _)| name.clone())
                .collect()
        };

        for worker_name in worker_names {
            let bare_name = if project.is_empty() {
                worker_name.clone()
            } else {
                worker_name
                    .strip_prefix(&format!("{}/", project))
                    .unwrap_or(&worker_name)
                    .to_string()
            };
            result_events.extend(
                self.executor
                    .execute_all(vec![Effect::Emit {
                        event: Event::WorkerWake { worker: bare_name, project: project.clone() },
                    }])
                    .await?,
            );
        }

        Ok(result_events)
    }
}
