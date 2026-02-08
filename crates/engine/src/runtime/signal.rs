// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent signal handling for explicit agent completion/escalation events.

use super::Runtime;
use crate::decision_builder::{EscalationDecisionBuilder, EscalationTrigger};
use crate::error::RuntimeError;
use crate::monitor;
use oj_adapters::agent::find_session_log;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{AgentId, AgentSignalKind, Clock, Effect, Event, Job, JobId, OwnerId, TimerId};

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Handle agent:signal event - agent explicitly signaling completion
    pub(crate) async fn handle_agent_done(
        &self,
        agent_id: &AgentId,
        kind: AgentSignalKind,
        message: Option<String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let Some(owner) = self.get_agent_owner(agent_id) else {
            tracing::warn!(agent_id = %agent_id, "agent:signal for unknown agent");
            return Ok(vec![]);
        };

        // Capture terminal snapshot before processing the signal
        self.capture_agent_terminal(agent_id).await;

        match owner {
            OwnerId::AgentRun(agent_run_id) => {
                let agent_run =
                    self.lock_state(|s| s.agent_runs.get(agent_run_id.as_str()).cloned());
                if let Some(agent_run) = agent_run {
                    return self
                        .handle_standalone_agent_done(agent_id, &agent_run, kind, message)
                        .await;
                }
                Ok(vec![])
            }
            OwnerId::Job(job_id) => {
                let job = self.require_job(job_id.as_str())?;
                if job.is_terminal() {
                    return Ok(vec![]);
                }

                // Verify this signal is for the current step's agent, not a stale signal
                // from a previous step's agent.
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
                        "dropping stale agent signal (agent_id mismatch)"
                    );
                    return Ok(vec![]);
                }

                self.handle_job_agent_done(&job, &job_id, kind, message)
                    .await
            }
        }
    }

    /// Handle agent:signal for job-owned agents
    async fn handle_job_agent_done(
        &self,
        job: &Job,
        job_id: &JobId,
        kind: AgentSignalKind,
        message: Option<String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        match kind {
            AgentSignalKind::Complete => {
                // Agent explicitly signaled completion — always advance the job.
                // This overrides gate escalation (StepStatus::Waiting) because the
                // agent's explicit signal is authoritative; the gate may have failed
                // due to environmental issues (e.g. shared target dir race).
                tracing::info!(job_id = %job.id, "agent:signal complete");
                self.logger
                    .append(&job.id, &job.step, "agent:signal complete");

                // Emit agent on_done notification
                if let Ok(runbook) = self.cached_runbook(&job.runbook_hash) {
                    if let Ok(agent_def) = crate::monitor::get_agent_def(&runbook, job) {
                        if let Some(effect) = monitor::build_agent_notify_effect(
                            job,
                            agent_def,
                            agent_def.notify.on_done.as_ref(),
                        ) {
                            self.executor.execute(effect).await?;
                        }
                    }
                }

                self.advance_job(job).await
            }
            AgentSignalKind::Continue => {
                tracing::info!(job_id = %job.id, "agent:signal continue");
                self.logger
                    .append(&job.id, &job.step, "agent:signal continue");
                Ok(vec![])
            }
            AgentSignalKind::Escalate => {
                let msg = message
                    .as_deref()
                    .unwrap_or("Agent requested escalation")
                    .to_string();
                tracing::info!(job_id = %job.id, message = %msg, "agent:signal escalate");
                self.logger
                    .append(&job.id, &job.step, &format!("agent:signal: {}", msg));

                let trigger = EscalationTrigger::Signal {
                    message: msg.clone(),
                };
                let (decision_id, decision_event) =
                    EscalationDecisionBuilder::for_job(job_id.clone(), job.name.clone(), trigger)
                        .agent_id(job.session_id.clone().unwrap_or_default())
                        .namespace(job.namespace.clone())
                        .build();

                let effects = vec![
                    Effect::Emit {
                        event: decision_event,
                    },
                    Effect::Notify {
                        title: format!("Job needs attention: {}", job.name),
                        message: msg,
                    },
                    Effect::Emit {
                        event: Event::StepWaiting {
                            job_id: job_id.clone(),
                            step: job.step.clone(),
                            reason: Some("agent:signal escalate".to_string()),
                            decision_id: Some(decision_id),
                        },
                    },
                    // Cancel exit-deferred timer (agent is still alive; liveness continues)
                    Effect::CancelTimer {
                        id: TimerId::exit_deferred(job_id),
                    },
                ];
                Ok(self.executor.execute_all(effects).await?)
            }
        }
    }

    /// Capture agent terminal output and save to the agent's log directory.
    ///
    /// Best-effort: failures are logged but do not interrupt signal handling.
    pub(crate) async fn capture_agent_terminal(&self, agent_id: &AgentId) {
        // Look up the agent's tmux session_id from state
        let session_id = self.lock_state(|s| {
            s.agents
                .get(agent_id.as_str())
                .and_then(|r| r.session_id.clone())
        });

        let Some(session_id) = session_id else {
            tracing::debug!(
                agent_id = %agent_id,
                "no session_id for agent, skipping terminal capture"
            );
            return;
        };

        match self.executor.capture_session_output(&session_id, 200).await {
            Ok(output) => {
                self.logger.write_agent_capture(agent_id.as_str(), &output);
            }
            Err(e) => {
                tracing::debug!(
                    agent_id = %agent_id,
                    session_id,
                    error = %e,
                    "failed to capture agent terminal"
                );
            }
        }
    }

    /// Copy the agent's session.jsonl to the logs directory on exit.
    ///
    /// Finds the session log from Claude's state directory and copies it to
    /// `{logs}/agent/{agent_id}/session.jsonl` for archival.
    pub(crate) fn copy_agent_session_log(&self, job: &Job) {
        // Get agent_id from step history
        let agent_id = match job
            .step_history
            .iter()
            .rfind(|r| r.name == job.step)
            .and_then(|r| r.agent_id.as_ref())
        {
            Some(id) => id,
            None => {
                tracing::debug!(
                    job_id = %job.id,
                    "no agent_id in step history, skipping session log copy"
                );
                return;
            }
        };

        // Get workspace path
        let workspace_path = self.execution_dir(job);

        // Find the session.jsonl
        if let Some(source) = find_session_log(&workspace_path, agent_id) {
            self.logger.copy_session_log(agent_id, &source);
        } else {
            tracing::debug!(
                job_id = %job.id,
                agent_id,
                workspace = %workspace_path.display(),
                "session.jsonl not found, skipping copy"
            );
        }
    }
}
