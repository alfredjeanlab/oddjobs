// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent state change handling

use super::super::Runtime;
use crate::error::RuntimeError;
use crate::monitor::{self, MonitorState};
use oj_adapters::agent::find_session_log;
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
enum OwnerCtx {
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
    fn get_owner_context(&self, agent_id: &AgentId) -> OwnerCtx {
        let Some(owner) = self.get_agent_owner(agent_id) else {
            return OwnerCtx::Unknown;
        };

        match owner {
            OwnerId::Job(job_id) => {
                // Get the job from state
                let Some(job) = self.get_job(job_id.as_str()) else {
                    return OwnerCtx::Unknown;
                };

                // Skip if terminal
                if job.is_terminal() {
                    return OwnerCtx::Skip;
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
                    return OwnerCtx::Skip;
                }

                OwnerCtx::Job {
                    job: Box::new(job),
                    job_id,
                }
            }
            OwnerId::AgentRun(agent_run_id) => {
                // Get the agent run from state
                let Some(agent_run) =
                    self.lock_state(|s| s.agent_runs.get(agent_run_id.as_str()).cloned())
                else {
                    return OwnerCtx::Unknown;
                };

                // Skip if terminal
                if agent_run.status.is_terminal() {
                    return OwnerCtx::Skip;
                }

                // Verify agent_id matches
                if agent_run.agent_id.as_deref() != Some(agent_id.as_str()) {
                    tracing::debug!(
                        agent_id = %agent_id,
                        agent_run_id = %agent_run.id,
                        "dropping stale standalone agent event (agent_id mismatch)"
                    );
                    return OwnerCtx::Skip;
                }

                OwnerCtx::AgentRun {
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
            OwnerCtx::Job { job, .. } => {
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
            OwnerCtx::AgentRun { agent_run, .. } => {
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
            OwnerCtx::Skip => Ok(vec![]),
            OwnerCtx::Unknown => {
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
            OwnerCtx::Job { job, job_id } => {
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
            OwnerCtx::AgentRun {
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
            OwnerCtx::Skip => Ok(vec![]),
            OwnerCtx::Unknown => {
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
        assistant_context: Option<&str>,
    ) -> Result<Vec<Event>, RuntimeError> {
        match self.get_owner_context(agent_id) {
            OwnerCtx::Job { job, .. } => {
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
                        assistant_context: assistant_context.map(|s| s.to_string()),
                    },
                )
                .await
            }
            OwnerCtx::AgentRun { agent_run, .. } => {
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
                        assistant_context: assistant_context.map(|s| s.to_string()),
                    },
                )
                .await
            }
            OwnerCtx::Skip => Ok(vec![]),
            OwnerCtx::Unknown => {
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
            OwnerCtx::Job { job, job_id } => {
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
            OwnerCtx::AgentRun {
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
            OwnerCtx::Skip | OwnerCtx::Unknown => Ok(vec![]),
        }
    }

    /// Handle resume for agent step: nudge if alive, recover if dead
    ///
    /// - If agent is alive and `kill` is false: nudge (send message to running agent)
    /// - If agent is alive and `kill` is true: kill session, then spawn with --resume
    /// - If agent is dead: spawn with --resume to continue conversation
    pub(crate) async fn handle_agent_resume(
        &self,
        job: &oj_core::Job,
        step: &str,
        agent_name: &str,
        message: &str,
        input: &HashMap<String, String>,
        kill: bool,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Collect all agent_ids from step history for this step (most recent first).
        // A step may have been retried multiple times, each with its own agent_id.
        let all_agent_ids: Vec<String> = job
            .step_history
            .iter()
            .rev()
            .filter(|r| r.name == step)
            .filter_map(|r| r.agent_id.clone())
            .collect();

        // The most recent agent_id is used to check if agent is alive
        let agent_id = all_agent_ids.first().map(AgentId::new);

        // Check if agent is alive (None means no agent_id, treat as dead)
        let agent_state = match &agent_id {
            Some(id) => self.executor.get_agent_state(id).await.ok(),
            None => None,
        };

        // If agent is alive and not killing, nudge it
        let is_alive = matches!(
            agent_state,
            Some(AgentState::Working) | Some(AgentState::WaitingForInput)
        );
        if !kill && is_alive {
            if let Some(id) = &agent_id {
                self.executor
                    .execute(Effect::SendToAgent {
                        agent_id: id.clone(),
                        input: message.to_string(),
                    })
                    .await?;

                // Update status to Running (preserve agent_id for the nudged agent)
                let job_id = JobId::new(&job.id);
                self.executor
                    .execute(Effect::Emit {
                        event: Event::StepStarted {
                            job_id: job_id.clone(),
                            step: step.to_string(),
                            agent_id: Some(id.clone()),
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
                return Ok(vec![]);
            }
        }

        // Agent dead OR --kill requested: recover using Claude's --resume to continue conversation
        let mut new_inputs = input.clone();
        new_inputs.insert("resume_message".to_string(), message.to_string());

        // Kill old tmux session if it exists (cleanup - Claude conversation persists in JSONL)
        if let Some(session_id) = &job.session_id {
            let _ = self
                .executor
                .execute(Effect::KillSession {
                    session_id: SessionId::new(session_id),
                })
                .await;
        }

        // Find a previous agent_id that has a valid session file to resume from.
        // Walk backwards through all agent_ids for this step (most recent first).
        // This recovers context even after failed resume attempts that died before writing JSONL.
        let workspace_path = self.execution_dir(job);
        let resume_id = all_agent_ids
            .iter()
            .find(|id| find_session_log(&workspace_path, id).is_some());

        if resume_id.is_none() && !all_agent_ids.is_empty() {
            tracing::warn!(
                job_id = %job.id,
                tried = ?all_agent_ids,
                "no valid session file found for any previous agent, starting fresh"
            );
        }

        let job_id = JobId::new(&job.id);
        let result = self
            .spawn_agent_with_resume(
                &job_id,
                agent_name,
                &new_inputs,
                resume_id.map(|s| s.as_str()),
            )
            .await?;

        tracing::info!(job_id = %job.id, kill, ?resume_id, "resumed agent with --resume");
        Ok(result)
    }
}
