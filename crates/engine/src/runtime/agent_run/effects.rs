// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Standalone agent action effect execution

use super::SpawnAgentParams;
use crate::error::RuntimeError;
use crate::monitor::{self, ActionEffects};
use crate::runtime::Runtime;
use crate::ActionContext;
use oj_adapters::subprocess::{run_with_timeout, GATE_TIMEOUT};
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{AgentRun, AgentRunId, AgentRunStatus, Clock, Effect, Event, SessionId};
use oj_runbook::AgentDef;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
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
                if let Some(effect) = monitor::build_agent_run_notify_effect(
                    agent_run,
                    agent_def,
                    agent_def.notify.on_done.as_ref(),
                ) {
                    self.executor.execute(effect).await?;
                }
                self.terminate_agent_run(agent_run, AgentRunStatus::Completed, None)
                    .await
            }
            ActionEffects::FailAgentRun { error } => {
                let mut fail_run = agent_run.clone();
                fail_run.error = Some(error.clone());
                if let Some(effect) = monitor::build_agent_run_notify_effect(
                    &fail_run,
                    agent_def,
                    agent_def.notify.on_fail.as_ref(),
                ) {
                    self.executor.execute(effect).await?;
                }
                self.terminate_agent_run(agent_run, AgentRunStatus::Failed, Some(error))
                    .await
            }
            ActionEffects::Resume {
                kill_session,
                agent_name,
                input,
                resume_session_id,
                ..
            } => {
                // Capture terminal + session log before killing old session
                if kill_session.is_some() {
                    self.capture_before_kill_agent_run(agent_run).await;
                }
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
                self.execute_standalone_gate(agent_run, agent_def, &command)
                    .await
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

    /// Execute a gate command for a standalone agent run.
    async fn execute_standalone_gate(
        &self,
        agent_run: &AgentRun,
        agent_def: &AgentDef,
        command: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let agent_run_id = AgentRunId::new(&agent_run.id);

        // Interpolate command
        let mut vars = crate::vars::namespace_vars(&agent_run.vars);
        vars.insert("agent_run_id".to_string(), agent_run_id.to_string());
        vars.insert("name".to_string(), agent_run.command_name.clone());
        vars.insert("workspace".to_string(), agent_run.cwd.display().to_string());
        let command = oj_runbook::interpolate_shell(command, &vars);

        tracing::info!(
            agent_run_id = %agent_run.id,
            gate = %command,
            cwd = %agent_run.cwd.display(),
            "running gate command for standalone agent"
        );

        match run_standalone_gate_command(agent_run, &command).await {
            Ok(()) => {
                // Gate passed — complete
                if let Some(effect) = monitor::build_agent_run_notify_effect(
                    agent_run,
                    agent_def,
                    agent_def.notify.on_done.as_ref(),
                ) {
                    self.executor.execute(effect).await?;
                }
                self.terminate_agent_run(agent_run, AgentRunStatus::Completed, None)
                    .await
            }
            Err(gate_error) => {
                // Gate failed — escalate
                let escalate_config =
                    oj_runbook::ActionConfig::simple(oj_runbook::AgentAction::Escalate);
                let escalate_effects = monitor::build_action_effects_for_agent_run(
                    &ActionContext {
                        agent_def,
                        action_config: &escalate_config,
                        trigger: "gate_failed",
                        chain_pos: 0,
                        question_data: None,
                        assistant_context: None,
                    },
                    agent_run,
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
}

/// Run a gate command for a standalone agent.
async fn run_standalone_gate_command(agent_run: &AgentRun, command: &str) -> Result<(), String> {
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
