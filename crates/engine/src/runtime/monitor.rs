// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent monitoring and lifecycle

use super::Runtime;
use crate::decision_builder::{EscalationDecisionBuilder, EscalationTrigger};
use crate::error::RuntimeError;
use crate::monitor::{self, ActionEffects, MonitorState};
use oj_adapters::agent::find_session_log;
use oj_adapters::subprocess::{run_with_timeout, GATE_TIMEOUT};
use oj_adapters::AgentReconnectConfig;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{
    AgentId, AgentSignalKind, Clock, Effect, Event, Job, JobId, OwnerId, PromptType, QuestionData,
    SessionId, TimerId,
};
use std::collections::HashMap;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Reconnect monitoring for an agent that survived a daemon restart.
    ///
    /// Registers the agent→job mapping and calls reconnect on the adapter.
    /// Does NOT spawn a new session — the tmux session must already be alive.
    pub async fn recover_agent(&self, job: &Job) -> Result<(), RuntimeError> {
        // Get agent_id from current step record (stored when agent was spawned)
        let agent_id_str = job
            .step_history
            .iter()
            .rfind(|r| r.name == job.step)
            .and_then(|r| r.agent_id.clone())
            .ok_or_else(|| {
                RuntimeError::JobNotFound(format!(
                    "job {} step {} has no agent_id",
                    job.id, job.step
                ))
            })?;
        let agent_id = AgentId::new(agent_id_str);

        let session_id = job.session_id.as_ref().ok_or_else(|| {
            RuntimeError::JobNotFound(format!("job {} has no session_id", job.id))
        })?;
        let workspace_path = self.execution_dir(job);

        // Register agent -> job mapping
        let job_id = JobId::new(&job.id);
        self.register_agent(agent_id.clone(), OwnerId::job(job_id.clone()));

        // Extract process_name from the runbook's agent definition
        let process_name = self
            .cached_runbook(&job.runbook_hash)
            .ok()
            .and_then(|rb| {
                crate::monitor::get_agent_def(&rb, job)
                    .ok()
                    .map(|def| oj_adapters::extract_process_name(&def.run))
            })
            .unwrap_or_else(|| "claude".to_string());

        // Reconnect monitoring via adapter
        let config = AgentReconnectConfig {
            agent_id,
            session_id: session_id.clone(),
            workspace_path,
            process_name,
        };
        self.executor.reconnect_agent(config).await?;

        // Restore liveness timer
        let job_id = JobId::new(&job.id);
        self.executor
            .execute(Effect::SetTimer {
                id: TimerId::liveness(&job_id),
                duration: crate::spawn::LIVENESS_INTERVAL,
            })
            .await?;

        Ok(())
    }

    pub(crate) async fn spawn_agent(
        &self,
        job_id: &JobId,
        agent_name: &str,
        input: &HashMap<String, String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        self.spawn_agent_with_resume(job_id, agent_name, input, None)
            .await
    }

    pub(crate) async fn spawn_agent_with_resume(
        &self,
        job_id: &JobId,
        agent_name: &str,
        input: &HashMap<String, String>,
        resume_session_id: Option<&str>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let job = self.require_job(job_id.as_str())?;
        let runbook = self.cached_runbook(&job.runbook_hash)?;
        let agent_def = runbook
            .get_agent(agent_name)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.to_string()))?;
        let execution_dir = self.execution_dir(&job);

        let ctx = crate::spawn::SpawnContext::from_job(&job, job_id);
        let mut effects = crate::spawn::build_spawn_effects(
            agent_def,
            &ctx,
            agent_name,
            input,
            &execution_dir,
            &self.state_dir,
            resume_session_id,
        )?;

        // Extract agent_id from SpawnAgent effect
        let agent_id = effects.iter().find_map(|e| match e {
            Effect::SpawnAgent { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        });

        // Log agent spawned with command
        let command = effects.iter().find_map(|e| match e {
            Effect::SpawnAgent { command, .. } => Some(command.as_str()),
            _ => None,
        });
        if let Some(cmd) = command {
            self.logger.append(
                job_id.as_str(),
                &job.step,
                &format!("agent spawned: {} ({})", agent_name, cmd),
            );
        }

        // Register agent -> job mapping for AgentStateChanged handling
        if let Some(ref aid) = agent_id {
            self.register_agent(aid.clone(), OwnerId::job(job_id.clone()));

            // Persist agent_id to WAL via StepStarted event (for daemon crash recovery)
            effects.push(Effect::Emit {
                event: Event::StepStarted {
                    job_id: job_id.clone(),
                    step: job.step.clone(),
                    agent_id: Some(aid.clone()),
                    agent_name: Some(agent_name.to_string()),
                },
            });

            // Log pointer to agent log in job log
            self.logger
                .append_agent_pointer(job_id.as_str(), &job.step, aid.as_str());
        }

        let mut result_events = self.executor.execute_all(effects).await?;

        // Emit agent on_start notification if configured
        if let Some(effect) =
            monitor::build_agent_notify_effect(&job, agent_def, agent_def.notify.on_start.as_ref())
        {
            if let Some(event) = self.executor.execute(effect).await? {
                result_events.push(event);
            }
        }

        Ok(result_events)
    }

    pub(crate) async fn handle_monitor_state(
        &self,
        job: &Job,
        agent_def: &oj_runbook::AgentDef,
        state: MonitorState,
    ) -> Result<Vec<Event>, RuntimeError> {
        let (action_config, trigger, qd) = match state {
            MonitorState::Working => {
                // Cancel idle grace timer — agent is working
                let job_id = JobId::new(&job.id);
                self.executor
                    .execute(Effect::CancelTimer {
                        id: TimerId::idle_grace(&job_id),
                    })
                    .await?;

                // Clear idle grace state
                self.lock_state_mut(|state| {
                    if let Some(p) = state.jobs.get_mut(job_id.as_str()) {
                        p.idle_grace_log_size = None;
                    }
                });

                if job.step_status.is_waiting() {
                    // Don't auto-resume within 60s of nudge — "Working" is
                    // likely from our own nudge text, not genuine progress
                    if let Some(nudge_at) = job.last_nudge_at {
                        let now = self.clock().epoch_ms();
                        if now.saturating_sub(nudge_at) < 60_000 {
                            tracing::debug!(
                                job_id = %job.id,
                                "suppressing auto-resume within 60s of nudge"
                            );
                            return Ok(vec![]);
                        }
                    }

                    tracing::info!(
                        job_id = %job.id,
                        step = %job.step,
                        "agent active, auto-resuming from escalation"
                    );
                    self.logger.append(
                        &job.id,
                        &job.step,
                        "agent active, auto-resuming from escalation",
                    );

                    let effects = vec![Effect::Emit {
                        event: Event::StepStarted {
                            job_id: job_id.clone(),
                            step: job.step.clone(),
                            agent_id: None,
                            agent_name: None,
                        },
                    }];

                    // Reset action attempts — agent demonstrated progress
                    self.lock_state_mut(|state| {
                        if let Some(p) = state.jobs.get_mut(job_id.as_str()) {
                            p.reset_action_attempts();
                        }
                    });

                    return Ok(self.executor.execute_all(effects).await?);
                }
                return Ok(vec![]);
            }
            MonitorState::WaitingForInput => {
                tracing::info!(job_id = %job.id, step = %job.step, "agent idle (on_idle)");
                self.logger.append(&job.id, &job.step, "agent idle");
                (&agent_def.on_idle, "idle", None)
            }
            MonitorState::Prompting {
                ref prompt_type,
                ref question_data,
            } => {
                tracing::info!(
                    job_id = %job.id,
                    prompt_type = ?prompt_type,
                    "agent prompting (on_prompt)"
                );
                self.logger.append(
                    &job.id,
                    &job.step,
                    &format!("agent prompt: {:?}", prompt_type),
                );
                // Use distinct trigger strings so escalation can differentiate
                let trigger_str = match prompt_type {
                    PromptType::Question => "prompt:question",
                    _ => "prompt",
                };
                (&agent_def.on_prompt, trigger_str, question_data.clone())
            }
            MonitorState::Failed {
                ref message,
                ref error_type,
            } => {
                tracing::warn!(job_id = %job.id, error = %message, "agent error");
                self.logger
                    .append(&job.id, &job.step, &format!("agent error: {}", message));
                // Write error to agent log so it's visible in `oj logs <agent>`
                if let Some(agent_id) = job
                    .step_history
                    .iter()
                    .rfind(|r| r.name == job.step)
                    .and_then(|r| r.agent_id.as_deref())
                {
                    self.logger.append_agent_error(agent_id, message);
                }
                let error_action = agent_def.on_error.action_for(error_type.as_ref());
                return self
                    .execute_action_with_attempts(job, agent_def, &error_action, message, 0, None)
                    .await;
            }
            MonitorState::Exited => {
                tracing::info!(job_id = %job.id, "agent process exited");
                self.logger.append(&job.id, &job.step, "agent exited");
                self.copy_agent_session_log(job);
                (&agent_def.on_dead, "exit", None)
            }
            MonitorState::Gone => {
                // Session gone is the normal exit path when tmux closes after
                // the agent process exits. Treat it the same as Exited so
                // that on_dead actions (done, gate, escalate, etc.) fire.
                tracing::info!(job_id = %job.id, "agent session ended");
                self.logger
                    .append(&job.id, &job.step, "agent session ended");
                self.copy_agent_session_log(job);
                (&agent_def.on_dead, "exit", None)
            }
        };

        self.execute_action_with_attempts(job, agent_def, action_config, trigger, 0, qd.as_ref())
            .await
    }

    /// Execute an action with attempt tracking and cooldown support
    pub(crate) async fn execute_action_with_attempts(
        &self,
        job: &Job,
        agent_def: &oj_runbook::AgentDef,
        action_config: &oj_runbook::ActionConfig,
        trigger: &str,
        chain_pos: usize,
        question_data: Option<&QuestionData>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let attempts = action_config.attempts();
        let job_id = JobId::new(&job.id);

        // Increment attempt count and get new value
        let attempt_num = self.lock_state_mut(|state| {
            state
                .jobs
                .get_mut(job_id.as_str())
                .map(|p| p.increment_action_attempt(trigger, chain_pos))
                .unwrap_or(1)
        });

        tracing::debug!(
            job_id = %job.id,
            trigger,
            chain_pos,
            attempt_num,
            max_attempts = ?attempts,
            "checking action attempts"
        );

        // Check if attempts exhausted (compare against attempt count BEFORE this attempt)
        if attempts.is_exhausted(attempt_num - 1) {
            tracing::info!(
                job_id = %job.id,
                trigger,
                attempts = attempt_num - 1,
                "attempts exhausted, escalating"
            );
            self.logger.append(
                &job.id,
                &job.step,
                &format!("{} attempts exhausted, escalating", trigger),
            );
            // Escalate
            let escalate_config =
                oj_runbook::ActionConfig::simple(oj_runbook::AgentAction::Escalate);
            return self
                .execute_action_effects(
                    job,
                    agent_def,
                    monitor::build_action_effects(
                        job,
                        agent_def,
                        &escalate_config,
                        &format!("{}_exhausted", trigger),
                        &job.vars,
                        question_data,
                    )?,
                )
                .await;
        }

        // Check if cooldown needed (not first attempt, cooldown configured)
        if attempt_num > 1 {
            if let Some(cooldown_str) = action_config.cooldown() {
                let duration = monitor::parse_duration(cooldown_str).map_err(|e| {
                    RuntimeError::InvalidRequest(format!(
                        "invalid cooldown '{}': {}",
                        cooldown_str, e
                    ))
                })?;
                let timer_id = TimerId::cooldown(&job_id, trigger, chain_pos);

                tracing::info!(
                    job_id = %job.id,
                    trigger,
                    attempt = attempt_num,
                    cooldown = ?duration,
                    "scheduling cooldown before retry"
                );
                self.logger.append(
                    &job.id,
                    &job.step,
                    &format!(
                        "{} attempt {} cooldown {:?}",
                        trigger, attempt_num, duration
                    ),
                );

                // Set cooldown timer - action will fire when timer expires
                self.executor
                    .execute(Effect::SetTimer {
                        id: timer_id,
                        duration,
                    })
                    .await?;

                return Ok(vec![]);
            }
        }

        // Execute the action
        self.execute_action_effects(
            job,
            agent_def,
            monitor::build_action_effects(
                job,
                agent_def,
                action_config,
                trigger,
                &job.vars,
                question_data,
            )?,
        )
        .await
    }

    /// Run a shell gate command for the `gate` on_dead action.
    ///
    /// The command should already be interpolated before calling this function.
    /// Returns `Ok(())` if the command exits successfully (exit code 0),
    /// `Err(message)` otherwise with a description of the failure including stderr.
    async fn run_gate_command(
        &self,
        job: &Job,
        command: &str,
        execution_dir: &std::path::Path,
    ) -> Result<(), String> {
        tracing::info!(
            job_id = %job.id,
            gate = %command,
            cwd = %execution_dir.display(),
            "running gate command"
        );

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command).current_dir(execution_dir);

        match run_with_timeout(cmd, GATE_TIMEOUT, "gate command").await {
            Ok(output) if output.status.success() => {
                tracing::info!(
                    job_id = %job.id,
                    gate = %command,
                    "gate passed, advancing job"
                );
                Ok(())
            }
            Ok(output) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::info!(
                    job_id = %job.id,
                    gate = %command,
                    exit_code,
                    stderr = %stderr,
                    "gate failed, escalating"
                );
                let stderr_trimmed = stderr.trim();
                let error = if stderr_trimmed.is_empty() {
                    format!("gate `{}` failed (exit {})", command, exit_code)
                } else {
                    format!(
                        "gate `{}` failed (exit {}): {}",
                        command, exit_code, stderr_trimmed
                    )
                };
                Err(error)
            }
            Err(e) => {
                tracing::warn!(
                    job_id = %job.id,
                    error = %e,
                    "gate execution error, escalating"
                );
                Err(format!("gate `{}` execution error: {}", command, e))
            }
        }
    }

    pub(crate) async fn execute_action_effects(
        &self,
        job: &Job,
        agent_def: &oj_runbook::AgentDef,
        effects: ActionEffects,
    ) -> Result<Vec<Event>, RuntimeError> {
        match effects {
            ActionEffects::Nudge { effects } => {
                self.logger.append(&job.id, &job.step, "nudge sent");

                // Record nudge timestamp to suppress auto-resume from our own nudge text
                let job_id = JobId::new(&job.id);
                let now = self.clock().epoch_ms();
                self.lock_state_mut(|state| {
                    if let Some(p) = state.jobs.get_mut(job_id.as_str()) {
                        p.last_nudge_at = Some(now);
                    }
                });

                Ok(self.executor.execute_all(effects).await?)
            }
            ActionEffects::AdvanceJob => {
                // Emit agent on_done notification before advancing
                if let Some(effect) = monitor::build_agent_notify_effect(
                    job,
                    agent_def,
                    agent_def.notify.on_done.as_ref(),
                ) {
                    self.executor.execute(effect).await?;
                }
                self.advance_job(job).await
            }
            ActionEffects::FailJob { error } => {
                // Emit agent on_fail notification before failing
                // Use the error from the FailJob variant since job.error
                // may not be set yet at this point
                let mut fail_job = job.clone();
                fail_job.error = Some(error.clone());
                if let Some(effect) = monitor::build_agent_notify_effect(
                    &fail_job,
                    agent_def,
                    agent_def.notify.on_fail.as_ref(),
                ) {
                    self.executor.execute(effect).await?;
                }
                self.fail_job(job, &error).await
            }
            ActionEffects::Resume {
                kill_session,
                agent_name,
                input,
                resume_session_id,
                ..
            } => {
                let job_id = JobId::new(&job.id);
                let session_id = kill_session.map(SessionId::new);
                self.kill_and_resume(
                    session_id,
                    &job_id,
                    &agent_name,
                    &input,
                    resume_session_id.as_deref(),
                )
                .await
            }
            ActionEffects::Escalate { effects } => Ok(self.executor.execute_all(effects).await?),
            ActionEffects::Gate { command } => {
                // Interpolate command before logging and execution
                let execution_dir = self.execution_dir(job);
                let job_id = JobId::new(&job.id);

                // Namespace job vars under "var." prefix (matching spawn.rs)
                let mut vars: HashMap<String, String> = job
                    .vars
                    .iter()
                    .map(|(k, v)| (format!("var.{}", k), v.clone()))
                    .collect();

                // Add system variables (not namespaced - these are always available)
                vars.insert("job_id".to_string(), job_id.to_string());
                vars.insert("name".to_string(), job.name.clone());
                vars.insert("workspace".to_string(), execution_dir.display().to_string());

                let command = oj_runbook::interpolate_shell(&command, &vars);

                self.logger.append(
                    &job.id,
                    &job.step,
                    &format!("gate (cwd: {}): {}", execution_dir.display(), command),
                );
                match self.run_gate_command(job, &command, &execution_dir).await {
                    Ok(()) => {
                        self.logger
                            .append(&job.id, &job.step, "gate passed, advancing");
                        // Emit agent on_done notification on gate pass
                        if let Some(effect) = monitor::build_agent_notify_effect(
                            job,
                            agent_def,
                            agent_def.notify.on_done.as_ref(),
                        ) {
                            self.executor.execute(effect).await?;
                        }
                        self.advance_job(job).await
                    }
                    Err(gate_error) => {
                        self.logger.append(
                            &job.id,
                            &job.step,
                            &format!("gate failed: {}", gate_error),
                        );

                        // Parse gate error for structured context
                        let (exit_code, stderr) = parse_gate_error(&gate_error);

                        // Create decision with gate failure context
                        let (_decision_id, decision_event) = EscalationDecisionBuilder::new(
                            job_id.clone(),
                            job.name.clone(),
                            EscalationTrigger::GateFailed {
                                command: command.clone(),
                                exit_code,
                                stderr,
                            },
                        )
                        .agent_id(job.session_id.clone().unwrap_or_default())
                        .namespace(job.namespace.clone())
                        .build();

                        let effects = vec![
                            Effect::Emit {
                                event: decision_event,
                            },
                            Effect::CancelTimer {
                                id: TimerId::exit_deferred(&job_id),
                            },
                        ];

                        Ok(self.executor.execute_all(effects).await?)
                    }
                }
            }
            // Standalone agent run effects should not be routed here
            ActionEffects::CompleteAgentRun
            | ActionEffects::FailAgentRun { .. }
            | ActionEffects::EscalateAgentRun { .. } => {
                tracing::error!(
                    job_id = %job.id,
                    "standalone agent action effect routed to job handler"
                );
                Ok(vec![])
            }
        }
    }

    async fn kill_and_resume(
        &self,
        kill_session: Option<SessionId>,
        job_id: &JobId,
        agent_name: &str,
        input: &HashMap<String, String>,
        resume_session_id: Option<&str>,
    ) -> Result<Vec<Event>, RuntimeError> {
        if let Some(sid) = kill_session {
            self.executor
                .execute(Effect::KillSession { session_id: sid })
                .await?;
        }
        self.spawn_agent_with_resume(job_id, agent_name, input, resume_session_id)
            .await
    }

    /// Copy the agent's session.jsonl to the logs directory on exit.
    ///
    /// Finds the session log from Claude's state directory and copies it to
    /// `{logs}/agent/{agent_id}/session.jsonl` for archival.
    fn copy_agent_session_log(&self, job: &Job) {
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
                let msg = message.as_deref().unwrap_or("Agent requested escalation");
                tracing::info!(job_id = %job.id, message = msg, "agent:signal escalate");
                self.logger
                    .append(&job.id, &job.step, &format!("agent:signal: {}", msg));
                let effects = vec![
                    Effect::Notify {
                        title: job.name.clone(),
                        message: msg.to_string(),
                    },
                    Effect::Emit {
                        event: Event::StepWaiting {
                            job_id: job_id.clone(),
                            step: job.step.clone(),
                            reason: Some(msg.to_string()),
                            decision_id: None,
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
}

/// Parse a gate error string into exit code and stderr.
///
/// The `run_gate_command` function produces errors in the format:
/// - `"gate `cmd` failed (exit N)"` - without stderr
/// - `"gate `cmd` failed (exit N): stderr_content"` - with stderr
/// - `"gate `cmd` execution error: msg"` - for spawn failures
fn parse_gate_error(error: &str) -> (i32, String) {
    // Try to extract exit code from "(exit N)" pattern
    if let Some(exit_start) = error.find("(exit ") {
        let after_exit = &error[exit_start + 6..];
        if let Some(paren_end) = after_exit.find(')') {
            if let Ok(code) = after_exit[..paren_end].trim().parse::<i32>() {
                // Check if there's stderr after the closing paren
                let rest = &after_exit[paren_end + 1..];
                let stderr = if let Some(colon_pos) = rest.find(':') {
                    rest[colon_pos + 1..].trim().to_string()
                } else {
                    String::new()
                };
                return (code, stderr);
            }
        }
    }
    // Fallback: unknown exit code, full string as stderr
    (1, error.to_string())
}
