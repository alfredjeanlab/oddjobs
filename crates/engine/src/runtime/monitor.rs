// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent monitoring and lifecycle

use super::Runtime;
use crate::decision_builder::{EscalationDecisionBuilder, EscalationTrigger};
use crate::error::RuntimeError;
use crate::monitor::{self, ActionEffects, MonitorState};
use crate::ActionContext;
use oj_adapters::subprocess::{run_with_timeout, GATE_TIMEOUT};
use oj_adapters::AgentReconnectConfig;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{AgentId, Clock, Effect, Event, Job, JobId, OwnerId, PromptType, SessionId, TimerId};
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
            owner: OwnerId::job(job_id.clone()),
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

        let ctx = crate::spawn::SpawnCtx::from_job(&job, job_id);
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
        // Fetch assistant context: from MonitorState for prompts, from executor for other states
        let assistant_context: Option<String> = match &state {
            MonitorState::Prompting {
                assistant_context, ..
            } => assistant_context.clone(),
            MonitorState::Working => None,
            _ => match step_agent_id(job) {
                Some(id) => {
                    self.executor
                        .get_last_assistant_message(&AgentId::new(id))
                        .await
                }
                None => None,
            },
        };

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

                    let mut effects = vec![Effect::Emit {
                        event: Event::StepStarted {
                            job_id: job_id.clone(),
                            step: job.step.clone(),
                            agent_id: None,
                            agent_name: None,
                        },
                    }];

                    // Auto-dismiss pending decision for this job
                    if let oj_core::StepStatus::Waiting(Some(ref decision_id)) = job.step_status {
                        let resolved_at_ms = self.clock().epoch_ms();
                        effects.push(Effect::Emit {
                            event: Event::DecisionResolved {
                                id: decision_id.clone(),
                                chosen: None,
                                choices: vec![],
                                message: Some("auto-dismissed: agent became active".to_string()),
                                resolved_at_ms,
                                namespace: job.namespace.clone(),
                            },
                        });
                    }

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
                ref assistant_context,
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
                    PromptType::PlanApproval => "prompt:plan",
                    _ => "prompt",
                };
                // Prompt actions fire once per occurrence — no attempt tracking.
                // The handle_agent_prompt_hook guard already prevents re-firing
                // while a decision is pending.
                return self
                    .execute_action_effects(
                        job,
                        agent_def,
                        monitor::build_action_effects(
                            &ActionContext {
                                agent_def,
                                action_config: &agent_def.on_prompt,
                                trigger: trigger_str,
                                chain_pos: 0,
                                question_data: question_data.as_ref(),
                                assistant_context: assistant_context.as_deref(),
                            },
                            job,
                        )?,
                    )
                    .await;
            }
            MonitorState::Failed {
                ref message,
                ref error_type,
            } => {
                tracing::warn!(job_id = %job.id, error = %message, "agent error");
                self.logger
                    .append(&job.id, &job.step, &format!("agent error: {}", message));
                if let Some(agent_id) = step_agent_id(job) {
                    self.logger.append_agent_error(agent_id, message);
                }
                let error_action = agent_def.on_error.action_for(error_type.as_ref());
                return self
                    .execute_action_with_attempts(
                        job,
                        &ActionContext {
                            agent_def,
                            action_config: &error_action,
                            trigger: message,
                            chain_pos: 0,
                            question_data: None,
                            assistant_context: assistant_context.as_deref(),
                        },
                    )
                    .await;
            }
            MonitorState::Exited { exit_code } => {
                let msg = monitor::format_exit_message(exit_code);
                tracing::info!(job_id = %job.id, exit_code, "{}", msg);
                self.logger.append(&job.id, &job.step, &msg);
                if let Some(agent_id) = step_agent_id(job) {
                    self.logger.append_agent_error(agent_id, &msg);
                    self.capture_agent_terminal(&AgentId::new(agent_id)).await;
                    self.copy_session_log(agent_id, &self.execution_dir(job));
                }
                (&agent_def.on_dead, "exit", None)
            }
            MonitorState::Gone => {
                tracing::info!(job_id = %job.id, "agent session ended");
                self.logger
                    .append(&job.id, &job.step, "agent session ended");
                if let Some(agent_id) = step_agent_id(job) {
                    self.copy_session_log(agent_id, &self.execution_dir(job));
                }
                (&agent_def.on_dead, "exit", None)
            }
        };

        self.execute_action_with_attempts(
            job,
            &ActionContext {
                agent_def,
                action_config,
                trigger,
                chain_pos: 0,
                question_data: qd.as_ref(),
                assistant_context: assistant_context.as_deref(),
            },
        )
        .await
    }

    /// Execute an action with attempt tracking and cooldown support
    pub(crate) async fn execute_action_with_attempts(
        &self,
        job: &Job,
        ctx: &ActionContext<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let attempts = ctx.action_config.attempts();
        let job_id = JobId::new(&job.id);

        // Increment attempt count and get new value
        let attempt_num = self.lock_state_mut(|state| {
            state
                .jobs
                .get_mut(job_id.as_str())
                .map(|p| p.increment_action_attempt(ctx.trigger, ctx.chain_pos))
                .unwrap_or(1)
        });

        tracing::debug!(
            job_id = %job.id,
            trigger = ctx.trigger,
            chain_pos = ctx.chain_pos,
            attempt_num,
            max_attempts = ?attempts,
            "checking action attempts"
        );

        // Check if attempts exhausted (compare against attempt count BEFORE this attempt)
        if attempts.is_exhausted(attempt_num - 1) {
            tracing::info!(
                job_id = %job.id,
                trigger = ctx.trigger,
                attempts = attempt_num - 1,
                "attempts exhausted, escalating"
            );
            self.logger.append(
                &job.id,
                &job.step,
                &format!("{} attempts exhausted, escalating", ctx.trigger),
            );
            // Escalate
            let escalate_config =
                oj_runbook::ActionConfig::simple(oj_runbook::AgentAction::Escalate);
            let exhausted_trigger = format!("{}:exhausted", ctx.trigger);
            return self
                .execute_action_effects(
                    job,
                    ctx.agent_def,
                    monitor::build_action_effects(
                        &ActionContext {
                            action_config: &escalate_config,
                            trigger: &exhausted_trigger,
                            ..*ctx
                        },
                        job,
                    )?,
                )
                .await;
        }

        // Check if cooldown needed (not first attempt, cooldown configured)
        if attempt_num > 1 {
            if let Some(cooldown_str) = ctx.action_config.cooldown() {
                let duration = monitor::parse_duration(cooldown_str).map_err(|e| {
                    RuntimeError::InvalidRequest(format!(
                        "invalid cooldown '{}': {}",
                        cooldown_str, e
                    ))
                })?;
                let timer_id = TimerId::cooldown(&job_id, ctx.trigger, ctx.chain_pos);

                tracing::info!(
                    job_id = %job.id,
                    trigger = ctx.trigger,
                    attempt = attempt_num,
                    cooldown = ?duration,
                    "scheduling cooldown before retry"
                );
                self.logger.append(
                    &job.id,
                    &job.step,
                    &format!(
                        "{} attempt {} cooldown {:?}",
                        ctx.trigger, attempt_num, duration
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
        self.execute_action_effects(job, ctx.agent_def, monitor::build_action_effects(ctx, job)?)
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
                let session_id = kill_session.map(SessionId::new);
                self.kill_and_resume(
                    job,
                    session_id,
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

                let mut vars = crate::vars::namespace_vars(&job.vars);

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
                        let (_decision_id, decision_event) = EscalationDecisionBuilder::for_job(
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
        job: &Job,
        kill_session: Option<SessionId>,
        agent_name: &str,
        input: &HashMap<String, String>,
        resume_session_id: Option<&str>,
    ) -> Result<Vec<Event>, RuntimeError> {
        if let Some(sid) = kill_session {
            self.capture_before_kill_job(job).await;
            self.executor
                .execute(Effect::KillSession { session_id: sid })
                .await?;
        }
        let job_id = JobId::new(&job.id);
        self.spawn_agent_with_resume(&job_id, agent_name, input, resume_session_id)
            .await
    }
}

/// Get the agent_id for the current step from a job's step history.
pub(super) fn step_agent_id(job: &Job) -> Option<&str> {
    job.step_history
        .iter()
        .rfind(|r| r.name == job.step)
        .and_then(|r| r.agent_id.as_deref())
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

#[cfg(test)]
#[path = "monitor_tests.rs"]
mod tests;
