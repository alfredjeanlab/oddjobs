// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Standalone agent lifecycle state handling and attempt tracking

use crate::decision_builder::{EscalationDecisionBuilder, EscalationTrigger};
use crate::error::RuntimeError;
use crate::monitor::{self, MonitorState};
use crate::runtime::Runtime;
use crate::ActionContext;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{
    AgentId, AgentRun, AgentRunId, AgentRunStatus, AgentSignalKind, Clock, Effect, Event, TimerId,
};
use oj_runbook::AgentDef;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Handle lifecycle state change for a standalone agent.
    pub(crate) async fn handle_standalone_monitor_state(
        &self,
        agent_run: &AgentRun,
        agent_def: &AgentDef,
        state: MonitorState,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Fetch assistant context: from MonitorState for prompts, from executor for other states
        let assistant_context: Option<String> = match &state {
            MonitorState::Prompting {
                assistant_context, ..
            } => assistant_context.clone(),
            MonitorState::Working => None,
            _ => {
                // For idle, exited, gone, failed: fetch from agent adapter
                let agent_id = agent_run.agent_id.as_ref().map(AgentId::new);
                match agent_id {
                    Some(aid) => self.executor.get_last_assistant_message(&aid).await,
                    None => None,
                }
            }
        };

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

                if agent_run.status == AgentRunStatus::Escalated
                    || agent_run.status == AgentRunStatus::Waiting
                {
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

                    let mut effects = vec![Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: agent_run_id.clone(),
                            status: AgentRunStatus::Running,
                            reason: Some("agent active".to_string()),
                        },
                    }];

                    // Auto-dismiss pending decision for this agent run
                    let owner = oj_core::OwnerId::AgentRun(agent_run_id.clone());
                    let pending_decision_id = self.lock_state(|state| {
                        state
                            .decisions
                            .values()
                            .find(|d| d.owner == owner && !d.is_resolved())
                            .map(|d| (d.id.as_str().to_string(), d.namespace.clone()))
                    });
                    if let Some((decision_id, namespace)) = pending_decision_id {
                        let resolved_at_ms = self.clock().epoch_ms();
                        effects.push(Effect::Emit {
                            event: Event::DecisionResolved {
                                id: decision_id,
                                chosen: None,
                                choices: vec![],
                                message: Some("auto-dismissed: agent became active".to_string()),
                                resolved_at_ms,
                                namespace,
                            },
                        });
                    }

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
                ref assistant_context,
            } => {
                tracing::info!(
                    agent_run_id = %agent_run.id,
                    prompt_type = ?prompt_type,
                    "standalone agent prompting (on_prompt)"
                );
                let trigger_str = match prompt_type {
                    oj_core::PromptType::Question => "prompt:question",
                    oj_core::PromptType::PlanApproval => "prompt:plan",
                    _ => "prompt",
                };
                // Prompt actions fire once per occurrence — no attempt tracking.
                return self
                    .execute_standalone_action_effects(
                        agent_run,
                        agent_def,
                        monitor::build_action_effects_for_agent_run(
                            &ActionContext {
                                agent_def,
                                action_config: &agent_def.on_prompt,
                                trigger: trigger_str,
                                chain_pos: 0,
                                question_data: question_data.as_ref(),
                                assistant_context: assistant_context.as_deref(),
                            },
                            agent_run,
                        )?,
                    )
                    .await;
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

    /// Execute an action with attempt tracking for standalone agent runs.
    pub(crate) async fn execute_standalone_action_with_attempts(
        &self,
        agent_run: &AgentRun,
        ctx: &ActionContext<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let attempts = ctx.action_config.attempts();
        let agent_run_id = AgentRunId::new(&agent_run.id);

        // Increment attempt count
        let attempt_num = self.lock_state_mut(|state| {
            state
                .agent_runs
                .get_mut(agent_run_id.as_str())
                .map(|ar| ar.increment_action_attempt(ctx.trigger, ctx.chain_pos))
                .unwrap_or(1)
        });

        tracing::debug!(
            agent_run_id = %agent_run.id,
            trigger = ctx.trigger,
            chain_pos = ctx.chain_pos,
            attempt_num,
            max_attempts = ?attempts,
            "checking standalone action attempts"
        );

        // Check if attempts exhausted
        if attempts.is_exhausted(attempt_num - 1) {
            tracing::info!(
                agent_run_id = %agent_run.id,
                trigger = ctx.trigger,
                attempts = attempt_num - 1,
                "attempts exhausted, escalating standalone agent"
            );
            let escalate_config =
                oj_runbook::ActionConfig::simple(oj_runbook::AgentAction::Escalate);
            let exhausted_trigger = format!("{}:exhausted", ctx.trigger);
            return self
                .execute_standalone_action_effects(
                    agent_run,
                    ctx.agent_def,
                    monitor::build_action_effects_for_agent_run(
                        &ActionContext {
                            action_config: &escalate_config,
                            trigger: &exhausted_trigger,
                            ..*ctx
                        },
                        agent_run,
                    )?,
                )
                .await;
        }

        // Check if cooldown needed
        if attempt_num > 1 {
            if let Some(cooldown_str) = ctx.action_config.cooldown() {
                let duration = monitor::parse_duration(cooldown_str).map_err(|e| {
                    RuntimeError::InvalidRequest(format!(
                        "invalid cooldown '{}': {}",
                        cooldown_str, e
                    ))
                })?;
                let timer_id =
                    TimerId::cooldown_agent_run(&agent_run_id, ctx.trigger, ctx.chain_pos);

                tracing::info!(
                    agent_run_id = %agent_run.id,
                    trigger = ctx.trigger,
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
            ctx.agent_def,
            monitor::build_action_effects_for_agent_run(ctx, agent_run)?,
        )
        .await
    }

    /// Handle agent:signal event for a standalone agent.
    pub(crate) async fn handle_standalone_agent_done(
        &self,
        _agent_id: &AgentId,
        agent_run: &AgentRun,
        kind: AgentSignalKind,
        message: Option<String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        if agent_run.status.is_terminal() {
            return Ok(vec![]);
        }

        let agent_run_id = AgentRunId::new(&agent_run.id);
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

                self.terminate_agent_run(agent_run, AgentRunStatus::Completed, None)
                    .await
            }
            AgentSignalKind::Continue => {
                tracing::info!(agent_run_id = %agent_run.id, "standalone agent:signal continue");
                Ok(vec![])
            }
            AgentSignalKind::Escalate => {
                let msg = message
                    .as_deref()
                    .unwrap_or("Agent requested escalation")
                    .to_string();
                tracing::info!(
                    agent_run_id = %agent_run.id,
                    message = %msg,
                    "standalone agent:signal escalate"
                );

                let trigger = EscalationTrigger::Signal {
                    message: msg.clone(),
                };
                let (_, decision_event) = EscalationDecisionBuilder::for_agent_run(
                    agent_run_id.clone(),
                    agent_run.command_name.clone(),
                    trigger,
                )
                .agent_id(agent_run.agent_id.clone().unwrap_or_default())
                .namespace(agent_run.namespace.clone())
                .build();

                let effects = vec![
                    Effect::Emit {
                        event: decision_event,
                    },
                    Effect::Notify {
                        title: agent_run.command_name.clone(),
                        message: msg,
                    },
                    Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: agent_run_id.clone(),
                            status: AgentRunStatus::Escalated,
                            reason: Some("agent:signal escalate".to_string()),
                        },
                    },
                    // Cancel exit-deferred timer (agent is still alive; liveness continues)
                    Effect::CancelTimer {
                        id: TimerId::exit_deferred_agent_run(&agent_run_id),
                    },
                ];
                Ok(self.executor.execute_all(effects).await?)
            }
        }
    }
}
