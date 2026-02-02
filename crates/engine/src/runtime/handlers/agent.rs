// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent state change handling

use super::super::Runtime;
use crate::error::RuntimeError;
use crate::monitor::{self, MonitorState};
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{AgentId, AgentState, Clock, Effect, Event, PipelineId, SessionId, TimerId};
use std::collections::HashMap;

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

    /// Handle resume for agent step: nudge if alive, recover if dead
    pub(crate) async fn handle_agent_resume(
        &self,
        pipeline: &oj_core::Pipeline,
        _agent_name: &str,
        message: &str,
        input: &HashMap<String, String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Get agent_id from step history (it's a UUID stored when the agent was spawned)
        let agent_id = pipeline
            .step_history
            .iter()
            .rfind(|r| r.name == pipeline.step)
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
                            step: pipeline.step.clone(),
                            agent_id: None,
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
                    .start_step(&pipeline_id, &pipeline.step, &new_inputs, &execution_dir)
                    .await?;

                tracing::info!(pipeline_id = %pipeline.id, "recovered agent");
                Ok(result)
            }
        }
    }
}
