// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent state change handling

use super::super::Runtime;
use crate::error::RuntimeError;
use crate::monitor::{self, MonitorState};
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{
    AgentId, AgentRun, AgentRunId, AgentRunStatus, AgentState, Clock, Effect, Event, Job, JobId,
    OwnerId, PromptType, QuestionData, SessionId, TimerId,
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

/// Result of looking up an agent's owner context.
enum OwnerContext {
    /// Agent is owned by a job
    Job { job: Box<Job>, job_id: JobId },
    /// Agent is owned by a standalone agent run
    AgentRun {
        agent_run: Box<AgentRun>,
        agent_run_id: AgentRunId,
    },
    /// Owner found but should be skipped (terminal or stale)
    Skip,
    /// No owner registered for this agent_id
    Unknown,
}

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Look up and validate an agent's owner context by agent_id.
    ///
    /// Returns the owner (Job or AgentRun) if found and valid for processing,
    /// Skip if the owner is terminal or the agent_id is stale, or Unknown if
    /// no owner is registered.
    fn get_owner_context(&self, agent_id: &AgentId) -> OwnerContext {
        let Some(owner) = self.get_agent_owner(agent_id) else {
            return OwnerContext::Unknown;
        };

        match owner {
            OwnerId::Job(job_id) => {
                // Get the job from state
                let Some(job) = self.get_job(job_id.as_str()) else {
                    return OwnerContext::Unknown;
                };

                // Skip if terminal
                if job.is_terminal() {
                    return OwnerContext::Skip;
                }

                // Verify this event is for the current step's agent, not a stale event
                let current_agent_id = job
                    .step_history
                    .iter()
                    .rfind(|r| r.name == job.step)
                    .and_then(|r| r.agent_id.as_deref());
                if current_agent_id != Some(agent_id.as_str()) {
                    tracing::debug!(
                        agent_id = %agent_id,
                        job_id = %job.id,
                        step = %job.step,
                        current_agent = ?current_agent_id,
                        "dropping stale agent event (agent_id mismatch)"
                    );
                    return OwnerContext::Skip;
                }

                OwnerContext::Job {
                    job: Box::new(job),
                    job_id,
                }
            }
            OwnerId::AgentRun(agent_run_id) => {
                // Get the agent run from state
                let Some(agent_run) =
                    self.lock_state(|s| s.agent_runs.get(agent_run_id.as_str()).cloned())
                else {
                    return OwnerContext::Unknown;
                };

                // Skip if terminal
                if agent_run.status.is_terminal() {
                    return OwnerContext::Skip;
                }

                // Verify agent_id matches
                if agent_run.agent_id.as_deref() != Some(agent_id.as_str()) {
                    tracing::debug!(
                        agent_id = %agent_id,
                        agent_run_id = %agent_run.id,
                        "dropping stale standalone agent event (agent_id mismatch)"
                    );
                    return OwnerContext::Skip;
                }

                OwnerContext::AgentRun {
                    agent_run: Box::new(agent_run),
                    agent_run_id,
                }
            }
        }
    }

    pub(crate) async fn handle_agent_state_changed(
        &self,
        agent_id: &oj_core::AgentId,
        state: &oj_core::AgentState,
    ) -> Result<Vec<Event>, RuntimeError> {
        match self.get_owner_context(agent_id) {
            OwnerContext::Job { job, .. } => {
                let runbook = self.cached_runbook(&job.runbook_hash)?;
                let agent_def = match monitor::get_agent_def(&runbook, &job) {
                    Ok(def) => def.clone(),
                    Err(_) => {
                        // Job already advanced past the agent step
                        return Ok(vec![]);
                    }
                };
                self.handle_monitor_state(&job, &agent_def, MonitorState::from_agent_state(state))
                    .await
            }
            OwnerContext::AgentRun { agent_run, .. } => {
                let runbook = self.cached_runbook(&agent_run.runbook_hash)?;
                let agent_def = runbook
                    .get_agent(&agent_run.agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_run.agent_name.clone()))?
                    .clone();
                self.handle_standalone_monitor_state(
                    &agent_run,
                    &agent_def,
                    MonitorState::from_agent_state(state),
                )
                .await
            }
            OwnerContext::Skip => Ok(vec![]),
            OwnerContext::Unknown => {
                tracing::warn!(agent_id = %agent_id, "received AgentStateChanged for unknown agent");
                Ok(vec![])
            }
        }
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
        match self.get_owner_context(agent_id) {
            OwnerContext::Job { job, job_id } => {
                // If job has a signal or is already waiting for a decision, ignore
                if job.action_tracker.agent_signal.is_some() || job.step_status.is_waiting() {
                    return Ok(vec![]);
                }

                // Deduplicate: if grace timer already pending, skip
                if job.idle_grace_log_size.is_some() {
                    tracing::debug!(
                        job_id = %job.id,
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
                self.lock_state_mut(|state| {
                    if let Some(p) = state.jobs.get_mut(job_id.as_str()) {
                        p.idle_grace_log_size = Some(log_size);
                    }
                });

                tracing::debug!(
                    job_id = %job.id,
                    log_size,
                    "setting idle grace timer"
                );
                self.executor
                    .execute(Effect::SetTimer {
                        id: TimerId::idle_grace(&job_id),
                        duration: idle_grace_period(),
                    })
                    .await?;

                Ok(vec![])
            }
            OwnerContext::AgentRun {
                agent_run,
                agent_run_id,
            } => {
                // Additional skip checks for idle handling
                if agent_run.action_tracker.agent_signal.is_some()
                    || agent_run.status == AgentRunStatus::Waiting
                    || agent_run.status == AgentRunStatus::Escalated
                {
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
                self.lock_state_mut(|state| {
                    if let Some(ar) = state.agent_runs.get_mut(agent_run_id.as_str()) {
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
                        id: TimerId::idle_grace_agent_run(&agent_run_id),
                        duration: idle_grace_period(),
                    })
                    .await?;

                Ok(vec![])
            }
            OwnerContext::Skip => Ok(vec![]),
            OwnerContext::Unknown => {
                tracing::debug!(agent_id = %agent_id, "agent:idle for unknown agent");
                Ok(vec![])
            }
        }
    }

    /// Handle agent:prompt from Notification hook
    pub(crate) async fn handle_agent_prompt_hook(
        &self,
        agent_id: &AgentId,
        prompt_type: &PromptType,
        question_data: Option<&QuestionData>,
    ) -> Result<Vec<Event>, RuntimeError> {
        match self.get_owner_context(agent_id) {
            OwnerContext::Job { job, .. } => {
                // If job has a signal or is already waiting for a decision, ignore
                if job.action_tracker.agent_signal.is_some() || job.step_status.is_waiting() {
                    return Ok(vec![]);
                }

                let runbook = self.cached_runbook(&job.runbook_hash)?;
                let agent_def = monitor::get_agent_def(&runbook, &job)?.clone();
                self.handle_monitor_state(
                    &job,
                    &agent_def,
                    MonitorState::Prompting {
                        prompt_type: prompt_type.clone(),
                        question_data: question_data.cloned(),
                    },
                )
                .await
            }
            OwnerContext::AgentRun { agent_run, .. } => {
                // Additional skip checks for prompt handling
                if agent_run.action_tracker.agent_signal.is_some()
                    || agent_run.status == AgentRunStatus::Waiting
                    || agent_run.status == AgentRunStatus::Escalated
                {
                    return Ok(vec![]);
                }
                let runbook = self.cached_runbook(&agent_run.runbook_hash)?;
                let agent_def = runbook
                    .get_agent(&agent_run.agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_run.agent_name.clone()))?
                    .clone();
                self.handle_standalone_monitor_state(
                    &agent_run,
                    &agent_def,
                    MonitorState::Prompting {
                        prompt_type: prompt_type.clone(),
                        question_data: question_data.cloned(),
                    },
                )
                .await
            }
            OwnerContext::Skip => Ok(vec![]),
            OwnerContext::Unknown => {
                tracing::debug!(agent_id = %agent_id, "agent:prompt for unknown agent");
                Ok(vec![])
            }
        }
    }

    /// Handle agent:stop — fired when on_stop=escalate and agent tries to exit.
    ///
    /// Escalates to human: sends notification and sets job/agent_run to waiting.
    /// Idempotent: skips if already in waiting/escalated status.
    pub(crate) async fn handle_agent_stop_hook(
        &self,
        agent_id: &AgentId,
    ) -> Result<Vec<Event>, RuntimeError> {
        match self.get_owner_context(agent_id) {
            OwnerContext::Job { job, job_id } => {
                if job.step_status.is_waiting() {
                    return Ok(vec![]); // Already escalated — no-op
                }

                let effects = vec![
                    Effect::Notify {
                        title: format!("Job needs attention: {}", job.name),
                        message: "Agent tried to stop without signaling completion".to_string(),
                    },
                    Effect::Emit {
                        event: Event::StepWaiting {
                            job_id: job_id.clone(),
                            step: job.step.clone(),
                            reason: Some("on_stop: escalate".to_string()),
                            decision_id: None,
                        },
                    },
                    Effect::CancelTimer {
                        id: TimerId::exit_deferred(&job_id),
                    },
                ];
                Ok(self.executor.execute_all(effects).await?)
            }
            OwnerContext::AgentRun {
                agent_run,
                agent_run_id,
            } => {
                // Additional skip check: already escalated
                if agent_run.status == AgentRunStatus::Escalated {
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
                            id: agent_run_id,
                            status: AgentRunStatus::Escalated,
                            reason: Some("on_stop: escalate".to_string()),
                        },
                    },
                ];
                Ok(self.executor.execute_all(effects).await?)
            }
            OwnerContext::Skip | OwnerContext::Unknown => Ok(vec![]),
        }
    }

    /// Handle resume for agent step: nudge if alive, recover if dead
    pub(crate) async fn handle_agent_resume(
        &self,
        job: &oj_core::Job,
        step: &str,
        _agent_name: &str,
        message: &str,
        input: &HashMap<String, String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Get agent_id from step history (it's a UUID stored when the agent was spawned)
        let agent_id = job
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

        // Match on (agent_state, agent_id) to satisfy clippy - agent_state is only Some when agent_id is Some
        match (agent_state, &agent_id) {
            (Some(AgentState::Working), Some(id))
            | (Some(AgentState::WaitingForInput), Some(id)) => {
                // Agent alive: nudge via SendToAgent (uses ClaudeAdapter::send with escape sequences)
                self.executor
                    .execute(Effect::SendToAgent {
                        agent_id: id.clone(),
                        input: message.to_string(),
                    })
                    .await?;

                // Update status to Running
                let job_id = JobId::new(&job.id);
                self.executor
                    .execute(Effect::Emit {
                        event: Event::StepStarted {
                            job_id: job_id.clone(),
                            step: step.to_string(),
                            agent_id: None,
                            agent_name: None,
                        },
                    })
                    .await?;

                // Restart liveness monitoring
                self.executor
                    .execute(Effect::SetTimer {
                        id: TimerId::liveness(&job_id),
                        duration: crate::spawn::LIVENESS_INTERVAL,
                    })
                    .await?;

                tracing::info!(job_id = %job.id, "nudged agent");
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
                if let Some(session_id) = &job.session_id {
                    let _ = self
                        .executor
                        .execute(Effect::KillSession {
                            session_id: SessionId::new(session_id),
                        })
                        .await;
                }

                // Re-spawn agent in same workspace
                let execution_dir = self.execution_dir(job);
                let job_id = JobId::new(&job.id);
                let result = self
                    .start_step(&job_id, step, &new_inputs, &execution_dir)
                    .await?;

                tracing::info!(job_id = %job.id, "resumed agent");
                Ok(result)
            }
        }
    }
}
