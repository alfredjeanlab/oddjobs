// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Standalone agent run lifecycle handling

use super::Runtime;
use crate::error::RuntimeError;
use crate::monitor::{self, ActionEffects, MonitorState};
use oj_adapters::agent::find_session_log;
use oj_adapters::subprocess::{run_with_timeout, GATE_TIMEOUT};
use oj_adapters::{AgentAdapter, AgentReconnectConfig, NotifyAdapter, SessionAdapter};
use oj_core::{
    AgentId, AgentRun, AgentRunId, AgentRunStatus, AgentSignalKind, Clock, Effect, Event, OwnerId,
    QuestionData, SessionId, TimerId,
};
use oj_runbook::AgentDef;
use std::collections::HashMap;
use std::path::Path;

/// Parameters for spawning a standalone agent.
pub(crate) struct SpawnAgentParams<'a> {
    pub agent_run_id: &'a AgentRunId,
    pub agent_def: &'a AgentDef,
    pub agent_name: &'a str,
    pub input: &'a HashMap<String, String>,
    pub cwd: &'a Path,
    pub namespace: &'a str,
    pub resume_session_id: Option<&'a str>,
}

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Spawn a standalone agent for a command run.
    ///
    /// Builds spawn effects using the agent definition, registers the agent→run
    /// mapping, and executes the effects. Returns events produced.
    pub(crate) async fn spawn_standalone_agent(
        &self,
        params: SpawnAgentParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let SpawnAgentParams {
            agent_run_id,
            agent_def,
            agent_name,
            input,
            cwd,
            namespace,
            resume_session_id,
        } = params;

        // Build a SpawnContext for standalone agent
        let ctx = crate::spawn::SpawnContext::for_agent_run(agent_run_id, agent_name, namespace);

        let effects = crate::spawn::build_spawn_effects(
            agent_def,
            &ctx,
            agent_name,
            input,
            cwd,
            &self.state_dir,
            resume_session_id,
        )?;

        // Extract agent_id from SpawnAgent effect
        let agent_id = effects.iter().find_map(|e| match e {
            Effect::SpawnAgent { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        });

        // Register agent → agent_run mapping
        if let Some(ref aid) = agent_id {
            self.register_agent(aid.clone(), OwnerId::agent_run(agent_run_id.clone()));
        }

        // Execute spawn effects, handling spawn failures gracefully
        let mut result_events = match self.executor.execute_all(effects).await {
            Ok(events) => events,
            Err(e) => {
                // Spawn failed - emit failure event and log the error
                let error_msg = e.to_string();
                tracing::error!(
                    agent_run_id = %agent_run_id,
                    error = %error_msg,
                    "standalone agent spawn failed"
                );

                // Write error to agent log (watcher isn't started, so we write directly)
                if let Some(ref aid) = agent_id {
                    self.logger.append_agent_error(aid.as_str(), &error_msg);
                }

                // Emit failure event so agent_run status is updated
                let fail_event = Event::AgentRunStatusChanged {
                    id: agent_run_id.clone(),
                    status: AgentRunStatus::Failed,
                    reason: Some(error_msg.clone()),
                };
                let _ = self
                    .executor
                    .execute(Effect::Emit { event: fail_event })
                    .await;

                // Return the original error so callers know spawn failed
                return Err(e.into());
            }
        };

        // Emit AgentRunStarted event if we have an agent_id
        if let Some(ref aid) = agent_id {
            let started_event = Event::AgentRunStarted {
                id: agent_run_id.clone(),
                agent_id: aid.clone(),
            };
            if let Some(ev) = self
                .executor
                .execute(Effect::Emit {
                    event: started_event,
                })
                .await?
            {
                result_events.push(ev);
            }
        }

        // Emit agent on_start notification if configured
        // Build a temporary AgentRun for notification context
        if let Some(effect) = agent_def.notify.on_start.as_ref().map(|template| {
            let mut vars: HashMap<String, String> = input
                .iter()
                .map(|(k, v)| (format!("var.{}", k), v.clone()))
                .collect();
            vars.insert("agent_run_id".to_string(), agent_run_id.to_string());
            vars.insert("name".to_string(), agent_name.to_string());
            vars.insert("agent".to_string(), agent_def.name.clone());
            let message = oj_runbook::NotifyConfig::render(template, &vars);
            Effect::Notify {
                title: agent_def.name.clone(),
                message,
            }
        }) {
            if let Some(ev) = self.executor.execute(effect).await? {
                result_events.push(ev);
            }
        }

        Ok(result_events)
    }

    /// Handle lifecycle state change for a standalone agent.
    pub(crate) async fn handle_standalone_monitor_state(
        &self,
        agent_run: &AgentRun,
        agent_def: &AgentDef,
        state: MonitorState,
    ) -> Result<Vec<Event>, RuntimeError> {
        let (action_config, trigger, qd) = match state {
            MonitorState::Working => {
                // Cancel idle grace timer — agent is working
                let agent_run_id = AgentRunId::new(&agent_run.id);
                self.executor
                    .execute(Effect::CancelTimer {
                        id: TimerId::idle_grace_agent_run(&agent_run_id),
                    })
                    .await?;

                // Clear idle grace state
                self.lock_state_mut(|state| {
                    if let Some(ar) = state.agent_runs.get_mut(agent_run_id.as_str()) {
                        ar.idle_grace_log_size = None;
                    }
                });

                if agent_run.status == AgentRunStatus::Escalated {
                    // Don't auto-resume within 60s of nudge
                    if let Some(nudge_at) = agent_run.last_nudge_at {
                        let now = self.clock().epoch_ms();
                        if now.saturating_sub(nudge_at) < 60_000 {
                            tracing::debug!(
                                agent_run_id = %agent_run.id,
                                "suppressing auto-resume within 60s of nudge"
                            );
                            return Ok(vec![]);
                        }
                    }

                    tracing::info!(
                        agent_run_id = %agent_run.id,
                        "standalone agent active, auto-resuming from escalation"
                    );

                    let effects = vec![Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: agent_run_id.clone(),
                            status: AgentRunStatus::Running,
                            reason: Some("agent active".to_string()),
                        },
                    }];

                    // Reset action attempts — agent demonstrated progress
                    self.lock_state_mut(|state| {
                        if let Some(ar) = state.agent_runs.get_mut(agent_run_id.as_str()) {
                            ar.reset_action_attempts();
                        }
                    });

                    return Ok(self.executor.execute_all(effects).await?);
                }
                return Ok(vec![]);
            }
            MonitorState::WaitingForInput => {
                tracing::info!(agent_run_id = %agent_run.id, "standalone agent idle (on_idle)");
                (&agent_def.on_idle, "idle", None)
            }
            MonitorState::Prompting {
                ref prompt_type,
                ref question_data,
            } => {
                tracing::info!(
                    agent_run_id = %agent_run.id,
                    prompt_type = ?prompt_type,
                    "standalone agent prompting (on_prompt)"
                );
                let trigger_str = match prompt_type {
                    oj_core::PromptType::Question => "prompt:question",
                    _ => "prompt",
                };
                (&agent_def.on_prompt, trigger_str, question_data.clone())
            }
            MonitorState::Failed {
                ref message,
                ref error_type,
            } => {
                tracing::warn!(agent_run_id = %agent_run.id, error = %message, "standalone agent error");
                // Write error to agent log so it's visible in `oj logs <agent>`
                if let Some(agent_id) = agent_run.agent_id.as_deref() {
                    self.logger.append_agent_error(agent_id, message);
                }
                let error_action = agent_def.on_error.action_for(error_type.as_ref());
                return self
                    .execute_standalone_action_with_attempts(
                        agent_run,
                        agent_def,
                        &error_action,
                        message,
                        0,
                        None,
                    )
                    .await;
            }
            MonitorState::Exited => {
                tracing::info!(agent_run_id = %agent_run.id, "standalone agent process exited");
                self.copy_standalone_agent_session_log(agent_run);
                (&agent_def.on_dead, "exit", None)
            }
            MonitorState::Gone => {
                tracing::info!(agent_run_id = %agent_run.id, "standalone agent session ended");
                self.copy_standalone_agent_session_log(agent_run);
                (&agent_def.on_dead, "exit", None)
            }
        };

        self.execute_standalone_action_with_attempts(
            agent_run,
            agent_def,
            action_config,
            trigger,
            0,
            qd.as_ref(),
        )
        .await
    }

    /// Execute an action with attempt tracking for standalone agent runs.
    pub(crate) async fn execute_standalone_action_with_attempts(
        &self,
        agent_run: &AgentRun,
        agent_def: &AgentDef,
        action_config: &oj_runbook::ActionConfig,
        trigger: &str,
        chain_pos: usize,
        question_data: Option<&QuestionData>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let attempts = action_config.attempts();
        let agent_run_id = AgentRunId::new(&agent_run.id);

        // Increment attempt count
        let attempt_num = self.lock_state_mut(|state| {
            state
                .agent_runs
                .get_mut(agent_run_id.as_str())
                .map(|ar| ar.increment_action_attempt(trigger, chain_pos))
                .unwrap_or(1)
        });

        tracing::debug!(
            agent_run_id = %agent_run.id,
            trigger,
            chain_pos,
            attempt_num,
            max_attempts = ?attempts,
            "checking standalone action attempts"
        );

        // Check if attempts exhausted
        if attempts.is_exhausted(attempt_num - 1) {
            tracing::info!(
                agent_run_id = %agent_run.id,
                trigger,
                attempts = attempt_num - 1,
                "attempts exhausted, escalating standalone agent"
            );
            let escalate_config =
                oj_runbook::ActionConfig::simple(oj_runbook::AgentAction::Escalate);
            return self
                .execute_standalone_action_effects(
                    agent_run,
                    agent_def,
                    monitor::build_action_effects_for_agent_run(
                        agent_run,
                        agent_def,
                        &escalate_config,
                        &format!("{}_exhausted", trigger),
                        &agent_run.vars,
                        question_data,
                    )?,
                )
                .await;
        }

        // Check if cooldown needed
        if attempt_num > 1 {
            if let Some(cooldown_str) = action_config.cooldown() {
                let duration = monitor::parse_duration(cooldown_str).map_err(|e| {
                    RuntimeError::InvalidRequest(format!(
                        "invalid cooldown '{}': {}",
                        cooldown_str, e
                    ))
                })?;
                let timer_id = TimerId::cooldown_agent_run(&agent_run_id, trigger, chain_pos);

                tracing::info!(
                    agent_run_id = %agent_run.id,
                    trigger,
                    attempt = attempt_num,
                    cooldown = ?duration,
                    "scheduling cooldown before standalone retry"
                );

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
        self.execute_standalone_action_effects(
            agent_run,
            agent_def,
            monitor::build_action_effects_for_agent_run(
                agent_run,
                agent_def,
                action_config,
                trigger,
                &agent_run.vars,
                question_data,
            )?,
        )
        .await
    }

    /// Execute action effects for a standalone agent run.
    pub(crate) async fn execute_standalone_action_effects(
        &self,
        agent_run: &AgentRun,
        agent_def: &AgentDef,
        effects: ActionEffects,
    ) -> Result<Vec<Event>, RuntimeError> {
        let agent_run_id = AgentRunId::new(&agent_run.id);

        match effects {
            ActionEffects::Nudge { effects } => {
                // Record nudge timestamp to suppress auto-resume from our own nudge text
                let now = self.clock().epoch_ms();
                self.lock_state_mut(|state| {
                    if let Some(ar) = state.agent_runs.get_mut(agent_run_id.as_str()) {
                        ar.last_nudge_at = Some(now);
                    }
                });
                Ok(self.executor.execute_all(effects).await?)
            }
            ActionEffects::CompleteAgentRun => {
                // Emit on_done notification
                if let Some(effect) = monitor::build_agent_run_notify_effect(
                    agent_run,
                    agent_def,
                    agent_def.notify.on_done.as_ref(),
                ) {
                    self.executor.execute(effect).await?;
                }

                let events = vec![
                    Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: agent_run_id.clone(),
                            status: AgentRunStatus::Completed,
                            reason: None,
                        },
                    },
                    Effect::CancelTimer {
                        id: TimerId::liveness_agent_run(&agent_run_id),
                    },
                ];
                let result = self.executor.execute_all(events).await?;

                // Kill the tmux session (mirrors job advance_job behavior)
                self.cleanup_standalone_agent_session(agent_run).await?;

                Ok(result)
            }
            ActionEffects::FailAgentRun { error } => {
                // Emit on_fail notification
                let mut fail_run = agent_run.clone();
                fail_run.error = Some(error.clone());
                if let Some(effect) = monitor::build_agent_run_notify_effect(
                    &fail_run,
                    agent_def,
                    agent_def.notify.on_fail.as_ref(),
                ) {
                    self.executor.execute(effect).await?;
                }

                let events = vec![
                    Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: agent_run_id.clone(),
                            status: AgentRunStatus::Failed,
                            reason: Some(error),
                        },
                    },
                    Effect::CancelTimer {
                        id: TimerId::liveness_agent_run(&agent_run_id),
                    },
                ];
                let result = self.executor.execute_all(events).await?;

                // Kill the tmux session (mirrors job fail_job behavior)
                self.cleanup_standalone_agent_session(agent_run).await?;

                Ok(result)
            }
            ActionEffects::Resume {
                kill_session,
                agent_name,
                input,
                resume_session_id,
                ..
            } => {
                // Kill old session if present
                if let Some(sid) = kill_session {
                    let _ = self
                        .executor
                        .execute(Effect::KillSession {
                            session_id: SessionId::new(sid),
                        })
                        .await;
                }
                // Re-spawn agent in same directory with resume support
                self.spawn_standalone_agent(SpawnAgentParams {
                    agent_run_id: &agent_run_id,
                    agent_def,
                    agent_name: &agent_name,
                    input: &input,
                    cwd: &agent_run.cwd,
                    namespace: &agent_run.namespace,
                    resume_session_id: resume_session_id.as_deref(),
                })
                .await
            }
            ActionEffects::EscalateAgentRun { effects } => {
                Ok(self.executor.execute_all(effects).await?)
            }
            ActionEffects::Gate { command } => {
                // Interpolate command
                let mut vars: HashMap<String, String> = agent_run
                    .vars
                    .iter()
                    .map(|(k, v)| (format!("var.{}", k), v.clone()))
                    .collect();
                vars.insert("agent_run_id".to_string(), agent_run_id.to_string());
                vars.insert("name".to_string(), agent_run.command_name.clone());
                vars.insert("workspace".to_string(), agent_run.cwd.display().to_string());

                let command = oj_runbook::interpolate_shell(&command, &vars);

                tracing::info!(
                    agent_run_id = %agent_run.id,
                    gate = %command,
                    cwd = %agent_run.cwd.display(),
                    "running gate command for standalone agent"
                );

                match self.run_standalone_gate_command(agent_run, &command).await {
                    Ok(()) => {
                        // Gate passed — complete
                        if let Some(effect) = monitor::build_agent_run_notify_effect(
                            agent_run,
                            agent_def,
                            agent_def.notify.on_done.as_ref(),
                        ) {
                            self.executor.execute(effect).await?;
                        }
                        let events = vec![
                            Effect::Emit {
                                event: Event::AgentRunStatusChanged {
                                    id: agent_run_id.clone(),
                                    status: AgentRunStatus::Completed,
                                    reason: None,
                                },
                            },
                            Effect::CancelTimer {
                                id: TimerId::liveness_agent_run(&agent_run_id),
                            },
                        ];
                        let result = self.executor.execute_all(events).await?;

                        // Kill the tmux session (mirrors job advance_job behavior)
                        self.cleanup_standalone_agent_session(agent_run).await?;

                        Ok(result)
                    }
                    Err(gate_error) => {
                        // Gate failed — escalate
                        let escalate_config =
                            oj_runbook::ActionConfig::simple(oj_runbook::AgentAction::Escalate);
                        let escalate_effects = monitor::build_action_effects_for_agent_run(
                            agent_run,
                            agent_def,
                            &escalate_config,
                            "gate_failed",
                            &agent_run.vars,
                            None,
                        )?;
                        match escalate_effects {
                            ActionEffects::EscalateAgentRun { effects } => {
                                let effects: Vec<_> = effects
                                    .into_iter()
                                    .map(|effect| match effect {
                                        Effect::Emit {
                                            event: Event::AgentRunStatusChanged { id, status, .. },
                                        } => Effect::Emit {
                                            event: Event::AgentRunStatusChanged {
                                                id,
                                                status,
                                                reason: Some(gate_error.clone()),
                                            },
                                        },
                                        other => other,
                                    })
                                    .collect();
                                Ok(self.executor.execute_all(effects).await?)
                            }
                            _ => unreachable!("escalate always produces EscalateAgentRun"),
                        }
                    }
                }
            }
            // Job-specific effects should not be routed here
            ActionEffects::AdvanceJob
            | ActionEffects::FailJob { .. }
            | ActionEffects::Escalate { .. } => {
                tracing::error!(
                    agent_run_id = %agent_run.id,
                    "job action effect routed to standalone agent handler"
                );
                Ok(vec![])
            }
        }
    }

    /// Run a gate command for a standalone agent.
    async fn run_standalone_gate_command(
        &self,
        agent_run: &AgentRun,
        command: &str,
    ) -> Result<(), String> {
        tracing::info!(
            agent_run_id = %agent_run.id,
            gate = %command,
            cwd = %agent_run.cwd.display(),
            "running standalone gate command"
        );

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command).current_dir(&agent_run.cwd);

        match run_with_timeout(cmd, GATE_TIMEOUT, "gate command").await {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&output.stderr);
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
            Err(e) => Err(format!("gate `{}` execution error: {}", command, e)),
        }
    }

    /// Kill the standalone agent's tmux session and clean up mappings.
    ///
    /// Mirrors what `advance_job` / `fail_job` do for job agents:
    /// deregister the agent mapping, kill the session, and emit `SessionDeleted`.
    async fn cleanup_standalone_agent_session(
        &self,
        agent_run: &AgentRun,
    ) -> Result<(), RuntimeError> {
        // Deregister agent → agent_run mapping so stale watcher events
        // from the dying session are dropped as unknown.
        if let Some(ref aid) = agent_run.agent_id {
            self.deregister_agent(&AgentId::new(aid));
        }

        // Kill the tmux session and emit SessionDeleted
        if let Some(ref session_id) = agent_run.session_id {
            let sid = SessionId::new(session_id);
            let _ = self
                .executor
                .execute(Effect::KillSession {
                    session_id: sid.clone(),
                })
                .await;
            let _ = self
                .executor
                .execute(Effect::Emit {
                    event: Event::SessionDeleted { id: sid },
                })
                .await;
        }

        Ok(())
    }

    /// Copy standalone agent session log on exit.
    fn copy_standalone_agent_session_log(&self, agent_run: &AgentRun) {
        let agent_id = match &agent_run.agent_id {
            Some(id) => id,
            None => return,
        };

        if let Some(source) = find_session_log(&agent_run.cwd, agent_id) {
            self.logger.copy_session_log(agent_id, &source);
        }
    }

    /// Handle agent:signal event for a standalone agent.
    pub(crate) async fn handle_standalone_agent_done(
        &self,
        _agent_id: &AgentId,
        agent_run: &AgentRun,
        kind: AgentSignalKind,
        message: Option<String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let agent_run_id = AgentRunId::new(&agent_run.id);

        if agent_run.status.is_terminal() {
            return Ok(vec![]);
        }

        match kind {
            AgentSignalKind::Complete => {
                tracing::info!(agent_run_id = %agent_run.id, "standalone agent:signal complete");

                // Copy session log before killing the session
                self.copy_standalone_agent_session_log(agent_run);

                // Emit on_done notification
                if let Ok(runbook) = self.cached_runbook(&agent_run.runbook_hash) {
                    if let Some(agent_def) = runbook.get_agent(&agent_run.agent_name) {
                        if let Some(effect) = monitor::build_agent_run_notify_effect(
                            agent_run,
                            agent_def,
                            agent_def.notify.on_done.as_ref(),
                        ) {
                            self.executor.execute(effect).await?;
                        }
                    }
                }

                let events = vec![
                    Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: agent_run_id.clone(),
                            status: AgentRunStatus::Completed,
                            reason: None,
                        },
                    },
                    Effect::CancelTimer {
                        id: TimerId::liveness_agent_run(&agent_run_id),
                    },
                ];
                let result = self.executor.execute_all(events).await?;

                // Kill the tmux session (mirrors job advance_job behavior)
                self.cleanup_standalone_agent_session(agent_run).await?;

                Ok(result)
            }
            AgentSignalKind::Continue => {
                tracing::info!(agent_run_id = %agent_run.id, "standalone agent:signal continue");
                Ok(vec![])
            }
            AgentSignalKind::Escalate => {
                let msg = message.as_deref().unwrap_or("Agent requested escalation");
                tracing::info!(
                    agent_run_id = %agent_run.id,
                    message = msg,
                    "standalone agent:signal escalate"
                );
                let events = vec![
                    Effect::Notify {
                        title: agent_run.command_name.clone(),
                        message: msg.to_string(),
                    },
                    Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: agent_run_id.clone(),
                            status: AgentRunStatus::Escalated,
                            reason: Some(msg.to_string()),
                        },
                    },
                    // Cancel exit-deferred timer (agent is still alive; liveness continues)
                    Effect::CancelTimer {
                        id: TimerId::exit_deferred_agent_run(&agent_run_id),
                    },
                ];
                Ok(self.executor.execute_all(events).await?)
            }
        }
    }

    /// Reconnect monitoring for a standalone agent that survived a daemon restart.
    pub async fn recover_standalone_agent(&self, agent_run: &AgentRun) -> Result<(), RuntimeError> {
        let agent_id_str = agent_run.agent_id.as_ref().ok_or_else(|| {
            RuntimeError::InvalidRequest(format!("agent_run {} has no agent_id", agent_run.id))
        })?;
        let agent_id = AgentId::new(agent_id_str);
        let agent_run_id = AgentRunId::new(&agent_run.id);

        let session_id = agent_run.session_id.as_ref().ok_or_else(|| {
            RuntimeError::InvalidRequest(format!("agent_run {} has no session_id", agent_run.id))
        })?;

        // Register agent → agent_run mapping
        self.register_agent(agent_id.clone(), OwnerId::agent_run(agent_run_id.clone()));

        // Extract process_name
        let process_name = self
            .cached_runbook(&agent_run.runbook_hash)
            .ok()
            .and_then(|rb| {
                rb.get_agent(&agent_run.agent_name)
                    .map(|def| oj_adapters::extract_process_name(&def.run))
            })
            .unwrap_or_else(|| "claude".to_string());

        let config = AgentReconnectConfig {
            agent_id,
            session_id: session_id.clone(),
            workspace_path: agent_run.cwd.clone(),
            process_name,
        };
        self.executor.reconnect_agent(config).await?;

        // Restore liveness timer
        self.executor
            .execute(Effect::SetTimer {
                id: TimerId::liveness_agent_run(&agent_run_id),
                duration: crate::spawn::LIVENESS_INTERVAL,
            })
            .await?;

        Ok(())
    }
}
