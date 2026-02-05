// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent state change handling

use super::super::Runtime;
use crate::error::RuntimeError;
use crate::monitor::{self, MonitorState};
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{
    AgentId, AgentRunId, AgentRunStatus, AgentState, Clock, Effect, Event, PipelineId, PromptType,
    QuestionData, SessionId, TimerId,
};
use std::collections::HashMap;
use std::time::Duration;

/// Grace period before acting on idle detection.
/// Prevents false idle triggers between tool calls.
/// Override with `OJ_IDLE_GRACE_MS` for integration tests.
pub(crate) fn idle_grace_period() -> Duration {
    match std::env::var("OJ_IDLE_GRACE_MS") {
        Ok(val) => Duration::from_millis(val.parse().unwrap_or(60_000)),
        Err(_) => Duration::from_secs(60),
    }
}

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_agent_state_changed(
        &self,
        agent_id: &oj_core::AgentId,
        state: &oj_core::AgentState,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Check standalone agent runs first
        let maybe_run_id = { self.agent_runs.lock().get(agent_id).cloned() };
        if let Some(agent_run_id) = maybe_run_id {
            let agent_run = self.lock_state(|s| s.agent_runs.get(agent_run_id.as_str()).cloned());
            if let Some(agent_run) = agent_run {
                if agent_run.status.is_terminal() {
                    return Ok(vec![]);
                }
                // Verify the agent_id matches
                if agent_run.agent_id.as_deref() != Some(agent_id.as_str()) {
                    tracing::debug!(
                        agent_id = %agent_id,
                        agent_run_id = %agent_run.id,
                        "dropping stale standalone agent event (agent_id mismatch)"
                    );
                    return Ok(vec![]);
                }
                let runbook = self.cached_runbook(&agent_run.runbook_hash)?;
                let agent_def = runbook
                    .get_agent(&agent_run.agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_run.agent_name.clone()))?
                    .clone();
                return self
                    .handle_standalone_monitor_state(
                        &agent_run,
                        &agent_def,
                        MonitorState::from_agent_state(state),
                    )
                    .await;
            }
        }

        // Look up pipeline ID for this agent
        let Some(pipeline_id) = self.agent_pipelines.lock().get(agent_id).cloned() else {
            tracing::warn!(agent_id = %agent_id, "received AgentStateChanged for unknown agent");
            return Ok(vec![]);
        };

        let pipeline = self.require_pipeline(&pipeline_id)?;

        if pipeline.is_terminal() {
            return Ok(vec![]);
        }

        // Verify this event is for the current step's agent, not a stale event
        // from a previous step's agent that hasn't been cleaned up yet.
        let current_agent_id = pipeline
            .step_history
            .iter()
            .rfind(|r| r.name == pipeline.step)
            .and_then(|r| r.agent_id.as_deref());
        if current_agent_id != Some(agent_id.as_str()) {
            tracing::debug!(
                agent_id = %agent_id,
                pipeline_id = %pipeline.id,
                step = %pipeline.step,
                current_agent = ?current_agent_id,
                "dropping stale agent event (agent_id mismatch)"
            );
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
        self.handle_monitor_state(&pipeline, &agent_def, MonitorState::from_agent_state(state))
            .await
    }

    /// Handle agent:idle from Notification hook
    ///
    /// Instead of acting immediately, sets a 60-second grace timer and records
    /// the current session log file size. When the timer fires, we check if the
    /// log has grown (any activity = not idle) and re-verify the agent state.
    pub(crate) async fn handle_agent_idle_hook(
        &self,
        agent_id: &AgentId,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Check standalone agent runs first
        let maybe_run_id = { self.agent_runs.lock().get(agent_id).cloned() };
        if let Some(agent_run_id) = maybe_run_id {
            let agent_run = self.lock_state(|s| s.agent_runs.get(agent_run_id.as_str()).cloned());
            if let Some(agent_run) = agent_run {
                if agent_run.status.is_terminal()
                    || agent_run.action_tracker.agent_signal.is_some()
                    || agent_run.status == AgentRunStatus::Waiting
                    || agent_run.status == AgentRunStatus::Escalated
                {
                    return Ok(vec![]);
                }
                if agent_run.agent_id.as_deref() != Some(agent_id.as_str()) {
                    return Ok(vec![]);
                }

                // Deduplicate: if grace timer already pending, skip
                if agent_run.idle_grace_log_size.is_some() {
                    tracing::debug!(
                        agent_run_id = %agent_run.id,
                        "idle grace timer already pending, deduplicating"
                    );
                    return Ok(vec![]);
                }

                // Record session log size and set grace timer
                let log_size = self
                    .executor
                    .get_session_log_size(agent_id)
                    .await
                    .unwrap_or(0);
                let ar_id = AgentRunId::new(&agent_run.id);
                self.lock_state_mut(|state| {
                    if let Some(ar) = state.agent_runs.get_mut(ar_id.as_str()) {
                        ar.idle_grace_log_size = Some(log_size);
                    }
                });

                tracing::debug!(
                    agent_run_id = %agent_run.id,
                    log_size,
                    "setting idle grace timer for standalone agent"
                );
                self.executor
                    .execute(Effect::SetTimer {
                        id: TimerId::idle_grace_agent_run(&ar_id),
                        duration: idle_grace_period(),
                    })
                    .await?;

                return Ok(vec![]);
            }
        }

        let Some(pipeline_id) = self.agent_pipelines.lock().get(agent_id).cloned() else {
            tracing::debug!(agent_id = %agent_id, "agent:idle for unknown agent");
            return Ok(vec![]);
        };

        let pipeline = self.require_pipeline(&pipeline_id)?;

        // If pipeline already advanced, has a signal, or is already waiting for a decision, ignore
        if pipeline.is_terminal()
            || pipeline.action_tracker.agent_signal.is_some()
            || pipeline.step_status.is_waiting()
        {
            return Ok(vec![]);
        }

        // Stale agent check
        let current_agent_id = pipeline
            .step_history
            .iter()
            .rfind(|r| r.name == pipeline.step)
            .and_then(|r| r.agent_id.as_deref());
        if current_agent_id != Some(agent_id.as_str()) {
            tracing::debug!(
                agent_id = %agent_id,
                pipeline_id = %pipeline.id,
                "dropping stale agent:idle (agent_id mismatch)"
            );
            return Ok(vec![]);
        }

        // Deduplicate: if grace timer already pending, skip
        if pipeline.idle_grace_log_size.is_some() {
            tracing::debug!(
                pipeline_id = %pipeline.id,
                "idle grace timer already pending, deduplicating"
            );
            return Ok(vec![]);
        }

        // Record session log size and set grace timer
        let log_size = self
            .executor
            .get_session_log_size(agent_id)
            .await
            .unwrap_or(0);
        let pid = PipelineId::new(&pipeline.id);
        self.lock_state_mut(|state| {
            if let Some(p) = state.pipelines.get_mut(pid.as_str()) {
                p.idle_grace_log_size = Some(log_size);
            }
        });

        tracing::debug!(
            pipeline_id = %pipeline.id,
            log_size,
            "setting idle grace timer"
        );
        self.executor
            .execute(Effect::SetTimer {
                id: TimerId::idle_grace(&pid),
                duration: idle_grace_period(),
            })
            .await?;

        Ok(vec![])
    }

    /// Handle agent:prompt from Notification hook
    pub(crate) async fn handle_agent_prompt_hook(
        &self,
        agent_id: &AgentId,
        prompt_type: &PromptType,
        question_data: Option<&QuestionData>,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Check standalone agent runs first
        let maybe_run_id = { self.agent_runs.lock().get(agent_id).cloned() };
        if let Some(agent_run_id) = maybe_run_id {
            let agent_run = self.lock_state(|s| s.agent_runs.get(agent_run_id.as_str()).cloned());
            if let Some(agent_run) = agent_run {
                if agent_run.status.is_terminal()
                    || agent_run.action_tracker.agent_signal.is_some()
                    || agent_run.status == AgentRunStatus::Waiting
                    || agent_run.status == AgentRunStatus::Escalated
                {
                    return Ok(vec![]);
                }
                if agent_run.agent_id.as_deref() != Some(agent_id.as_str()) {
                    return Ok(vec![]);
                }
                let runbook = self.cached_runbook(&agent_run.runbook_hash)?;
                let agent_def = runbook
                    .get_agent(&agent_run.agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_run.agent_name.clone()))?
                    .clone();
                return self
                    .handle_standalone_monitor_state(
                        &agent_run,
                        &agent_def,
                        MonitorState::Prompting {
                            prompt_type: prompt_type.clone(),
                            question_data: question_data.cloned(),
                        },
                    )
                    .await;
            }
        }

        let Some(pipeline_id) = self.agent_pipelines.lock().get(agent_id).cloned() else {
            tracing::debug!(agent_id = %agent_id, "agent:prompt for unknown agent");
            return Ok(vec![]);
        };

        let pipeline = self.require_pipeline(&pipeline_id)?;

        // If pipeline already advanced, has a signal, or is already waiting for a decision, ignore
        if pipeline.is_terminal()
            || pipeline.action_tracker.agent_signal.is_some()
            || pipeline.step_status.is_waiting()
        {
            return Ok(vec![]);
        }

        // Stale agent check
        let current_agent_id = pipeline
            .step_history
            .iter()
            .rfind(|r| r.name == pipeline.step)
            .and_then(|r| r.agent_id.as_deref());
        if current_agent_id != Some(agent_id.as_str()) {
            tracing::debug!(
                agent_id = %agent_id,
                pipeline_id = %pipeline.id,
                "dropping stale agent:prompt (agent_id mismatch)"
            );
            return Ok(vec![]);
        }

        let runbook = self.cached_runbook(&pipeline.runbook_hash)?;
        let agent_def = monitor::get_agent_def(&runbook, &pipeline)?.clone();
        self.handle_monitor_state(
            &pipeline,
            &agent_def,
            MonitorState::Prompting {
                prompt_type: prompt_type.clone(),
                question_data: question_data.cloned(),
            },
        )
        .await
    }

    /// Handle agent:stop — fired when on_stop=escalate and agent tries to exit.
    ///
    /// Escalates to human: sends notification and sets pipeline/agent_run to waiting.
    /// Idempotent: skips if already in waiting/escalated status.
    pub(crate) async fn handle_agent_stop_hook(
        &self,
        agent_id: &AgentId,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Check standalone agent runs first
        let maybe_run_id = { self.agent_runs.lock().get(agent_id).cloned() };
        if let Some(agent_run_id) = maybe_run_id {
            let agent_run = self.lock_state(|s| s.agent_runs.get(agent_run_id.as_str()).cloned());
            if let Some(agent_run) = agent_run {
                if agent_run.status.is_terminal() || agent_run.status == AgentRunStatus::Escalated {
                    return Ok(vec![]);
                }
                // Verify agent_id matches
                if agent_run.agent_id.as_deref() != Some(agent_id.as_str()) {
                    return Ok(vec![]);
                }
                // Fire standalone escalation
                let effects = vec![
                    Effect::Notify {
                        title: format!("Agent needs attention: {}", agent_run.command_name),
                        message: "Agent tried to stop without signaling completion".to_string(),
                    },
                    Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: AgentRunId::new(&agent_run.id),
                            status: AgentRunStatus::Escalated,
                            reason: Some("on_stop: escalate".to_string()),
                        },
                    },
                ];
                return Ok(self.executor.execute_all(effects).await?);
            }
        }

        // Pipeline agent
        let Some(pipeline_id_str) = self.agent_pipelines.lock().get(agent_id).cloned() else {
            return Ok(vec![]);
        };
        let pipeline = self.require_pipeline(&pipeline_id_str)?;

        if pipeline.is_terminal() || pipeline.step_status.is_waiting() {
            return Ok(vec![]); // Already escalated or terminal — no-op
        }

        // Stale agent check
        let current_agent_id = pipeline
            .step_history
            .iter()
            .rfind(|r| r.name == pipeline.step)
            .and_then(|r| r.agent_id.as_deref());
        if current_agent_id != Some(agent_id.as_str()) {
            return Ok(vec![]);
        }

        let pipeline_id = PipelineId::new(&pipeline.id);
        let effects = vec![
            Effect::Notify {
                title: format!("Pipeline needs attention: {}", pipeline.name),
                message: "Agent tried to stop without signaling completion".to_string(),
            },
            Effect::Emit {
                event: Event::StepWaiting {
                    pipeline_id: pipeline_id.clone(),
                    step: pipeline.step.clone(),
                    reason: Some("on_stop: escalate".to_string()),
                    decision_id: None,
                },
            },
            Effect::CancelTimer {
                id: TimerId::exit_deferred(&pipeline_id),
            },
        ];
        Ok(self.executor.execute_all(effects).await?)
    }

    /// Handle resume for agent step: nudge if alive, recover if dead
    pub(crate) async fn handle_agent_resume(
        &self,
        pipeline: &oj_core::Pipeline,
        step: &str,
        _agent_name: &str,
        message: &str,
        input: &HashMap<String, String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Get agent_id from step history (it's a UUID stored when the agent was spawned)
        let agent_id = pipeline
            .step_history
            .iter()
            .rfind(|r| r.name == step)
            .and_then(|r| r.agent_id.clone())
            .map(AgentId::new);

        // Check if agent is alive (None means no agent_id, treat as dead)
        let agent_state = match &agent_id {
            Some(id) => self.executor.get_agent_state(id).await.ok(),
            None => None,
        };

        match agent_state {
            Some(AgentState::Working) | Some(AgentState::WaitingForInput) => {
                // Agent alive: nudge
                let session_id = pipeline
                    .session_id
                    .as_ref()
                    .ok_or_else(|| RuntimeError::InvalidRequest("no session for nudge".into()))?;

                self.executor
                    .execute(Effect::SendToSession {
                        session_id: SessionId::new(session_id),
                        input: format!("{}\n", message),
                    })
                    .await?;

                // Update status to Running
                let pipeline_id = PipelineId::new(&pipeline.id);
                self.executor
                    .execute(Effect::Emit {
                        event: Event::StepStarted {
                            pipeline_id: pipeline_id.clone(),
                            step: step.to_string(),
                            agent_id: None,
                            agent_name: None,
                        },
                    })
                    .await?;

                // Restart liveness monitoring
                self.executor
                    .execute(Effect::SetTimer {
                        id: TimerId::liveness(&pipeline_id),
                        duration: crate::spawn::LIVENESS_INTERVAL,
                    })
                    .await?;

                tracing::info!(pipeline_id = %pipeline.id, "nudged agent");
                Ok(vec![])
            }
            _ => {
                // Agent dead: recover
                // Build modified input with message in prompt
                let mut new_inputs = input.clone();
                let existing_prompt = new_inputs.get("prompt").cloned().unwrap_or_default();
                new_inputs.insert(
                    "prompt".to_string(),
                    format!("{}\n\n{}", existing_prompt, message),
                );

                // Kill old session if it exists
                if let Some(session_id) = &pipeline.session_id {
                    let _ = self
                        .executor
                        .execute(Effect::KillSession {
                            session_id: SessionId::new(session_id),
                        })
                        .await;
                }

                // Re-spawn agent in same workspace
                let execution_dir = self.execution_dir(pipeline);
                let pipeline_id = PipelineId::new(&pipeline.id);
                let result = self
                    .start_step(&pipeline_id, step, &new_inputs, &execution_dir)
                    .await?;

                tracing::info!(pipeline_id = %pipeline.id, "resumed agent");
                Ok(result)
            }
        }
    }
}
