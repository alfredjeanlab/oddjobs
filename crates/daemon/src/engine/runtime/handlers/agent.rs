// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent state change handling

use super::super::Runtime;
use crate::adapters::{AgentAdapter, NotifyAdapter};
use crate::engine::error::RuntimeError;
use crate::engine::lifecycle::RunLifecycle;
use crate::engine::monitor::MonitorState;
use oj_core::{
    AgentId, AgentState, Clock, Crew, Effect, Event, Job, JobId, OwnerId, PromptType, QuestionData,
    TimerId,
};
use std::collections::HashMap;

/// Result of looking up an agent's owner context.
enum OwnerCtx {
    /// Agent is owned by a job
    Job { job: Box<Job> },
    /// Agent is owned by a crew
    Agent { agent: Box<Crew> },
    /// Owner found but should be skipped (terminal or stale)
    Skip,
    /// No owner registered for this agent_id
    Unknown,
}

impl OwnerCtx {
    /// Extract the RunLifecycle trait object, or None for Skip/Unknown.
    fn as_run(&self) -> Option<&dyn RunLifecycle> {
        match self {
            OwnerCtx::Job { job } => Some(job.as_ref()),
            OwnerCtx::Agent { agent } => Some(agent.as_ref()),
            OwnerCtx::Skip | OwnerCtx::Unknown => None,
        }
    }

    fn is_unknown(&self) -> bool {
        matches!(self, OwnerCtx::Unknown)
    }
}

impl<A, N, C> Runtime<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Look up and validate an agent's owner context by agent_id.
    ///
    /// Returns a resolved entity if found and valid for processing,
    /// Skip if the owner is terminal or the agent_id is stale, or Unknown if
    /// no owner is registered.
    fn get_owner_context(&self, agent_id: &AgentId) -> OwnerCtx {
        let Some(owner) = self.get_agent_owner(agent_id) else {
            return OwnerCtx::Unknown;
        };

        match owner {
            OwnerId::Job(job_id) => {
                let Some(job) = self.get_job(job_id.as_str()) else {
                    return OwnerCtx::Unknown;
                };
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
                    return OwnerCtx::Skip;
                }

                OwnerCtx::Job { job: Box::new(job) }
            }
            OwnerId::Crew(crew_id) => {
                let Some(crew) = self.lock_state(|s| s.crew.get(crew_id.as_str()).cloned()) else {
                    return OwnerCtx::Unknown;
                };
                if crew.status.is_terminal() {
                    return OwnerCtx::Skip;
                }

                // Verify agent_id matches
                if crew.agent_id.as_deref() != Some(agent_id.as_str()) {
                    return OwnerCtx::Skip;
                }

                OwnerCtx::Agent { agent: Box::new(crew) }
            }
        }
    }

    pub(crate) async fn handle_agent_status(
        &self,
        agent_id: &oj_core::AgentId,
        state: &oj_core::AgentState,
    ) -> Result<Vec<Event>, RuntimeError> {
        let ctx = self.get_owner_context(agent_id);
        let Some(run) = ctx.as_run() else {
            if ctx.is_unknown() {
                tracing::warn!(agent_id = %agent_id, "received AgentStateChanged for unknown agent");
            }
            return Ok(vec![]);
        };
        let runbook = self.cached_runbook(run.runbook_hash())?;
        let Ok(agent_def) = run.resolve_agent_def(&runbook) else {
            return Ok(vec![]); // Job advanced past the agent step
        };
        self.handle_monitor_state_for(run, &agent_def, MonitorState::from_agent_state(state)).await
    }

    /// Handle agent:idle — dispatches the on_idle action.
    ///
    /// This is the primary trigger for on_idle dispatch. For allow-mode actions
    /// (done/fail), this is the sole trigger. For gate-mode actions (nudge,
    /// gate, resume, escalate), stop:blocked may fire first as a faster path;
    /// the is_waiting() guard in handle_monitor_state_for prevents double-dispatch.
    /// For auto, the engine does not intervene.
    ///
    /// Print-mode agents may briefly transition through idle before exiting.
    /// A liveness check prevents false-positive idle dispatch — if the agent
    /// process is already dead, we skip dispatch and let the exit path (on_dead)
    /// handle it.
    pub(crate) async fn handle_agent_idle_hook(
        &self,
        agent_id: &AgentId,
    ) -> Result<Vec<Event>, RuntimeError> {
        let ctx = self.get_owner_context(agent_id);
        let Some(run) = ctx.as_run() else {
            return Ok(vec![]);
        };
        if run.is_waiting() {
            return Ok(vec![]);
        }

        // Guard: if the agent process is already dead, skip on_idle dispatch.
        // Print-mode agents (-p) briefly transition through idle before exiting;
        // dispatching on_idle here would race with the exit path (on_dead).
        if !self.executor.agents.is_alive(agent_id).await {
            return Ok(vec![]);
        }

        let runbook = self.cached_runbook(run.runbook_hash())?;
        let Ok(agent_def) = run.resolve_agent_def(&runbook) else {
            return Ok(vec![]);
        };

        // Auto mode: engine does not intervene (coop handles self-determination).
        let on_idle = agent_def
            .on_idle
            .as_ref()
            .map(|c| c.action().clone())
            .unwrap_or(oj_runbook::AgentAction::Escalate);
        if matches!(on_idle, oj_runbook::AgentAction::Auto) {
            return Ok(vec![]);
        }

        self.handle_monitor_state_for(run, &agent_def, MonitorState::WaitingForInput).await
    }

    /// Handle agent:prompt from Notification hook
    pub(crate) async fn handle_agent_prompt_hook(
        &self,
        agent_id: &AgentId,
        prompt_type: &PromptType,
        questions: Option<&QuestionData>,
        last_message: Option<&str>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let ctx = self.get_owner_context(agent_id);
        let Some(run) = ctx.as_run() else {
            return Ok(vec![]);
        };
        if run.is_waiting() {
            return Ok(vec![]);
        }
        let runbook = self.cached_runbook(run.runbook_hash())?;
        let agent_def = run.resolve_agent_def(&runbook)?;
        let monitor_state = MonitorState::Prompting {
            prompt_type: prompt_type.clone(),
            questions: questions.cloned(),
            last_message: last_message.map(|s| s.to_string()),
        };
        self.handle_monitor_state_for(run, &agent_def, monitor_state).await
    }

    /// Handle agent:stop:blocked — coop blocked the agent from stopping.
    ///
    /// Used for gate-mode actions (nudge, gate, resume, escalate).
    /// Resolves the stop (allows the agent to proceed) then dispatches on_idle.
    /// This is a fast path — AgentIdle also dispatches as a fallback.
    pub(crate) async fn handle_agent_stop_blocked(
        &self,
        agent_id: &AgentId,
    ) -> Result<Vec<Event>, RuntimeError> {
        let ctx = self.get_owner_context(agent_id);
        let Some(run) = ctx.as_run() else {
            return Ok(vec![]);
        };
        let runbook = self.cached_runbook(run.runbook_hash())?;
        let Ok(agent_def) = run.resolve_agent_def(&runbook) else {
            return Ok(vec![]);
        };

        // Resolve the stop so the agent can proceed
        self.executor.agents.resolve_stop(agent_id).await;

        // Dispatch on_idle via the unified monitor path
        self.handle_monitor_state_for(run, &agent_def, MonitorState::WaitingForInput).await
    }

    /// Handle agent:stop:allowed — coop allowed the turn to end naturally.
    ///
    /// Used for allow-mode actions (done, fail). The agent's turn ended
    /// without interception; dispatch on_idle to execute the configured action.
    /// This is a fast path — AgentIdle also dispatches as a fallback.
    pub(crate) async fn handle_agent_stop_allowed(
        &self,
        agent_id: &AgentId,
    ) -> Result<Vec<Event>, RuntimeError> {
        let ctx = self.get_owner_context(agent_id);
        let Some(run) = ctx.as_run() else {
            return Ok(vec![]);
        };
        let runbook = self.cached_runbook(run.runbook_hash())?;
        let Ok(agent_def) = run.resolve_agent_def(&runbook) else {
            return Ok(vec![]);
        };

        // Dispatch on_idle via the unified monitor path
        // No resolve_stop needed — coop already allowed the turn to end
        self.handle_monitor_state_for(run, &agent_def, MonitorState::WaitingForInput).await
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
        let is_alive =
            matches!(agent_state, Some(AgentState::Working) | Some(AgentState::WaitingForInput));
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
                        duration: crate::engine::spawn::LIVENESS_INTERVAL,
                    })
                    .await?;

                tracing::info!(job_id = %job.id, "nudged agent");
                return Ok(vec![]);
            }
        }

        // Agent dead OR --kill requested: recover using Claude's --resume to continue conversation
        let mut new_inputs = input.clone();
        new_inputs.insert("resume_message".to_string(), message.to_string());

        // When killing a live agent, emit JobSuspending as a WAL-durable intermediate
        // so the intent is recorded. If the daemon crashes between kill and respawn,
        // the job can be detected as suspending and resumed later.
        if kill && is_alive {
            self.executor
                .execute(Effect::Emit { event: Event::JobSuspending { id: JobId::new(&job.id) } })
                .await?;
        }

        // Capture terminal + session log before killing old session
        self.capture_before_kill_job(job).await;

        // Kill old agent if it exists (cleanup - Claude conversation persists in JSONL)
        if let Some(id) = all_agent_ids.first() {
            let _ = self.executor.execute(Effect::KillAgent { agent_id: AgentId::new(id) }).await;
        }

        // Resume with coop's --resume flag (coop discovers session ID from JSONL)
        let resume = !all_agent_ids.is_empty();
        let job_id = JobId::new(&job.id);
        let result = self.spawn_agent_with_resume(&job_id, agent_name, &new_inputs, resume).await?;

        tracing::info!(job_id = %job.id, kill, resume, "resumed agent with --resume");
        Ok(result)
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
