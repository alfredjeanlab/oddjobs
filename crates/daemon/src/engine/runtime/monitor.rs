// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::Runtime;
use crate::adapters::AgentReconnectConfig;
use crate::engine::decision::{EscalationDecisionBuilder, EscalationTrigger};
use crate::engine::error::RuntimeError;
use crate::engine::lifecycle::RunLifecycle;
use crate::engine::monitor::{self, ActionEffects, MonitorState};
use crate::engine::ActionContext;
use oj_core::{
    AgentId, Clock, CrewId, CrewStatus, DecisionId, Effect, Event, Job, JobId, OwnerId, PromptType,
    TimerId,
};
use std::collections::HashMap;

impl<C: Clock> Runtime<C> {
    /// Reconnect monitoring for an agent that survived a daemon restart.
    ///
    /// Registers the agent→job mapping and calls reconnect on the adapter.
    /// Does NOT spawn a new session — the coop process must already be alive.
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
        let agent_id = AgentId::new(&agent_id_str);

        // Look up persisted runtime and auth token from agent records
        let (runtime_hint, auth_token) = self.lock_state(|s| {
            s.agents
                .get(&agent_id_str)
                .map(|r| (r.runtime, r.auth_token.clone()))
                .unwrap_or_default()
        });

        // Register agent -> job mapping
        let job_id = JobId::new(&job.id);
        self.register_agent(agent_id.clone(), OwnerId::job(job_id.clone()));

        // Reconnect monitoring via adapter
        let config = AgentReconnectConfig {
            agent_id,
            owner: OwnerId::job(job_id.clone()),
            runtime_hint,
            auth_token,
        };
        self.executor.reconnect_agent(config).await?;

        // Restore liveness timer
        let job_id = JobId::new(&job.id);
        self.executor
            .execute(Effect::SetTimer {
                id: TimerId::liveness(&job_id),
                duration: crate::engine::spawn::LIVENESS_INTERVAL,
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
        self.spawn_agent_with_resume(job_id, agent_name, input, false).await
    }

    pub(crate) async fn spawn_agent_with_resume(
        &self,
        job_id: &JobId,
        agent_name: &str,
        input: &HashMap<String, String>,
        resume: bool,
    ) -> Result<Vec<Event>, RuntimeError> {
        let job = self.require_job(job_id.as_str())?;
        let runbook = self.cached_runbook(&job.runbook_hash)?;
        let agent_def = runbook
            .get_agent(agent_name)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.to_string()))?;
        let execution_dir = job.execution_dir().to_path_buf();

        let ctx = crate::engine::spawn::SpawnCtx::from_job(&job, job_id);
        let mut effects = crate::engine::spawn::build_spawn_effects(
            agent_def,
            &ctx,
            agent_name,
            input,
            &execution_dir,
            &self.state_dir,
            resume,
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
            self.logger.append_agent_pointer(job_id.as_str(), &job.step, aid.as_str());
        }

        let result_events = self.executor.execute_all(effects).await?;

        // on_start notification is emitted by handle_session_created() when
        // the background SpawnAgent task completes successfully.

        Ok(result_events)
    }

    /// Handle monitor state for any lifecycle entity.
    ///
    /// Unified across Job and Crew. Dispatches entity-specific effects via
    /// the `RunLifecycle` trait and OwnerId-based state mutation helpers.
    pub(crate) async fn handle_monitor_state_for(
        &self,
        run: &dyn RunLifecycle,
        agent_def: &oj_runbook::AgentDef,
        state: MonitorState,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Fetch assistant context: from MonitorState for prompts, from executor for other states
        let last_message: Option<String> = match &state {
            MonitorState::Prompting { last_message, .. } => last_message.clone(),
            MonitorState::Working => None,
            _ => match run.agent_id() {
                Some(id) => self.executor.agents.last_message(&AgentId::new(id)).await,
                None => None,
            },
        };

        let default_on_idle = oj_runbook::ActionConfig::default();
        let (action_config, trigger, qd) = match state {
            MonitorState::Working => {
                let owner = run.owner_id();

                if run.is_waiting() {
                    // Don't auto-resume within 60s of nudge — "Working" is
                    // likely from our own nudge text, not genuine progress
                    if let Some(nudge_at) = run.last_nudge_at() {
                        let now = self.executor.clock().epoch_ms();
                        if now.saturating_sub(nudge_at) < 60_000 {
                            return Ok(vec![]);
                        }
                    }

                    tracing::info!(
                        entity_id = %run.log_id(),
                        "agent active, auto-resuming from escalation"
                    );
                    self.log_entity_activity(run, "agent active, auto-resuming from escalation");

                    let resolved_at_ms = self.executor.clock().epoch_ms();
                    let mut effects = run.auto_resume_effects(resolved_at_ms);

                    // For crew, look up pending decision via owner (jobs embed
                    // the decision_id directly in auto_resume_effects via StepStatus).
                    if let OwnerId::Crew(_) = &owner {
                        let pending = self.lock_state(|state| {
                            state
                                .decisions
                                .values()
                                .find(|d| d.owner == owner && !d.is_resolved())
                                .map(|d| (d.id.as_str().to_string(), d.project.clone()))
                        });
                        if let Some((decision_id, project)) = pending {
                            effects.push(Effect::Emit {
                                event: Event::DecisionResolved {
                                    id: DecisionId::new(decision_id),
                                    choices: vec![],
                                    message: Some(
                                        "auto-dismissed: agent became active".to_string(),
                                    ),
                                    resolved_at_ms,
                                    project,
                                },
                            });
                        }
                    }

                    // Reset action attempts — agent demonstrated progress
                    self.reset_run_attempts(&owner);

                    return Ok(self.executor.execute_all(effects).await?);
                }
                return Ok(vec![]);
            }
            MonitorState::WaitingForInput => {
                tracing::info!(entity_id = %run.log_id(), "agent idle (on_idle)");
                self.log_entity_activity(run, "agent idle");
                (agent_def.on_idle.as_ref().unwrap_or(&default_on_idle), "idle", None)
            }
            MonitorState::Prompting { ref prompt_type, ref questions, ref last_message } => {
                tracing::info!(
                    entity_id = %run.log_id(),
                    prompt_type = ?prompt_type,
                    "agent prompting (on_prompt)"
                );
                self.log_entity_activity(run, &format!("agent prompt: {:?}", prompt_type));
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
                    .execute_action_effects_for(
                        run,
                        agent_def,
                        monitor::build_action_effects_for(
                            &ActionContext {
                                agent_def,
                                action_config: &agent_def.on_prompt,
                                trigger: trigger_str,
                                chain_pos: 0,
                                questions: questions.as_ref(),
                                last_message: last_message.as_deref(),
                            },
                            run,
                        )?,
                    )
                    .await;
            }
            MonitorState::Failed { ref message, ref error_type } => {
                tracing::warn!(entity_id = %run.log_id(), error = %message, "agent error");
                self.log_entity_activity(run, &format!("agent error: {}", message));
                if let Some(agent_id) = run.agent_id() {
                    self.logger.append_agent_error(agent_id, message);
                }
                let error_action = agent_def.on_error.action_for(error_type.as_ref());
                return self
                    .execute_action_with_attempts_for(
                        run,
                        &ActionContext {
                            agent_def,
                            action_config: &error_action,
                            trigger: message,
                            chain_pos: 0,
                            questions: None,
                            last_message: last_message.as_deref(),
                        },
                    )
                    .await;
            }
            MonitorState::Exited { exit_code } => {
                let msg = monitor::format_exit_message(exit_code);
                tracing::info!(entity_id = %run.log_id(), exit_code, "{}", msg);
                self.log_entity_activity(run, &msg);
                if let Some(agent_id) = run.agent_id() {
                    self.logger.append_agent_error(agent_id, &msg);
                    let aid = AgentId::new(agent_id);
                    self.capture_agent_terminal(&aid).await;
                    self.archive_session_transcript(&aid).await;
                }
                (&agent_def.on_dead, "exit", None)
            }
            MonitorState::Gone => {
                tracing::info!(entity_id = %run.log_id(), "agent session ended");
                self.log_entity_activity(run, "agent session ended");
                if let Some(agent_id) = run.agent_id() {
                    let aid = AgentId::new(agent_id);
                    self.capture_agent_terminal(&aid).await;
                    self.archive_session_transcript(&aid).await;
                }
                (&agent_def.on_dead, "exit", None)
            }
        };

        // Guard: don't execute on_dead/on_idle when a decision is already pending.
        // But if the pending decision is for an ALIVE agent and the agent is now DEAD,
        // auto-dismiss the stale decision and proceed with on_dead dispatch.
        //
        // The entity snapshot's is_waiting() may be stale (apply_event overwrites
        // step_status before handle_event runs), so we check the decision table directly.
        let owner = run.owner_id();
        if let Some((decision_id, decision_source)) = self.pending_decision_source(&owner) {
            let is_dead_trigger = trigger == "exit";
            if is_dead_trigger && decision_source.is_alive_agent_source() {
                // Agent died while an alive decision was pending — dismiss it.
                tracing::info!(
                    entity_id = %run.log_id(),
                    decision_id = %decision_id,
                    source = ?decision_source,
                    "auto-dismissing stale alive decision — agent exited"
                );
                let resolved_at_ms = self.executor.clock().epoch_ms();
                self.executor
                    .execute(Effect::Emit {
                        event: Event::DecisionResolved {
                            id: decision_id,
                            choices: vec![],
                            message: Some("auto-dismissed: agent exited".to_string()),
                            resolved_at_ms,
                            project: run.project().to_string(),
                        },
                    })
                    .await?;
            } else {
                // Pending decision is authoritative — skip action dispatch.
                return Ok(vec![]);
            }
        }

        self.execute_action_with_attempts_for(
            run,
            &ActionContext {
                agent_def,
                action_config,
                trigger,
                chain_pos: 0,
                questions: qd.as_ref(),
                last_message: last_message.as_deref(),
            },
        )
        .await
    }

    /// Execute an action with attempt tracking — generic over entity type.
    pub(crate) async fn execute_action_with_attempts_for(
        &self,
        run: &dyn RunLifecycle,
        ctx: &ActionContext<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let attempts = ctx.action_config.attempts();
        let owner = run.owner_id();

        // Increment attempt count and get new value
        let attempt_num = self.increment_run_attempt(&owner, ctx.trigger, ctx.chain_pos);

        // Check if attempts exhausted (compare against attempt count BEFORE this attempt)
        if attempts.is_exhausted(attempt_num - 1) {
            tracing::info!(
                entity_id = %run.log_id(),
                trigger = ctx.trigger,
                attempts = attempt_num - 1,
                "attempts exhausted, escalating"
            );
            let escalate_config =
                oj_runbook::ActionConfig::simple(oj_runbook::AgentAction::Escalate);
            let exhausted_trigger = format!("{}:exhausted", ctx.trigger);
            return self
                .execute_action_effects_for(
                    run,
                    ctx.agent_def,
                    monitor::build_action_effects_for(
                        &ActionContext {
                            action_config: &escalate_config,
                            trigger: &exhausted_trigger,
                            ..*ctx
                        },
                        run,
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
                let timer_id = TimerId::cooldown(&owner, ctx.trigger, ctx.chain_pos);

                tracing::info!(
                    entity_id = %run.log_id(),
                    trigger = ctx.trigger,
                    attempt = attempt_num,
                    cooldown = ?duration,
                    "scheduling cooldown before retry"
                );

                self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;

                return Ok(vec![]);
            }
        }

        // Execute the action
        self.execute_action_effects_for(
            run,
            ctx.agent_def,
            monitor::build_action_effects_for(ctx, run)?,
        )
        .await
    }

    /// Execute action effects — generic over entity type.
    ///
    /// Dispatches terminal actions (Advance, Fail, Resume, Gate) via OwnerId.
    /// Nudge and Escalate are entity-agnostic.
    pub(crate) async fn execute_action_effects_for(
        &self,
        run: &dyn RunLifecycle,
        agent_def: &oj_runbook::AgentDef,
        effects: ActionEffects,
    ) -> Result<Vec<Event>, RuntimeError> {
        match effects {
            ActionEffects::Nudge { effects } => {
                self.log_entity_activity(run, "nudge sent");
                let now = self.executor.clock().epoch_ms();
                self.set_run_nudge_at(&run.owner_id(), now);
                Ok(self.executor.execute_all(effects).await?)
            }
            ActionEffects::Advance => {
                // Emit on_done notification
                if let Some(effect) = crate::engine::lifecycle::notify_on_done(run, agent_def) {
                    self.executor.execute(effect).await?;
                }
                self.advance_run(run).await
            }
            ActionEffects::Fail { error } => {
                // Build notification with error context (error may not be set
                // on the entity yet, so inject it directly into vars)
                let mut vars = run.build_notify_vars(agent_def);
                vars.insert("error".to_string(), error.clone());
                if let Some(template) = agent_def.notify.on_fail.as_ref() {
                    let message = oj_runbook::NotifyConfig::render(template, &vars);
                    self.executor
                        .execute(Effect::Notify { title: agent_def.name.clone(), message })
                        .await?;
                }
                self.fail_run(run, &error).await
            }
            ActionEffects::Resume { kill_agent, agent_name, input, resume } => {
                self.resume_run(run, agent_def, kill_agent, &agent_name, &input, resume).await
            }
            ActionEffects::Escalate { effects } => Ok(self.executor.execute_all(effects).await?),
            ActionEffects::Gate { command } => self.run_gate(run, agent_def, &command).await,
        }
    }

    /// Advance (complete) an entity — Job advances to next step, Crew completes.
    async fn advance_run(&self, run: &dyn RunLifecycle) -> Result<Vec<Event>, RuntimeError> {
        match run.owner_id() {
            OwnerId::Job(job_id) => self.advance_job(&self.require_job(job_id.as_str())?).await,
            OwnerId::Crew(run_id) => {
                let run = self.require_crew(run_id.as_str())?;
                self.terminate_crew(&run, CrewStatus::Completed, None).await
            }
        }
    }

    /// Fail an entity — Job fails, Crew terminates with Failed status.
    async fn fail_run(
        &self,
        run: &dyn RunLifecycle,
        error: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        match run.owner_id() {
            OwnerId::Job(job_id) => self.fail_job(&self.require_job(job_id.as_str())?, error).await,
            OwnerId::Crew(run_id) => {
                let run = self.require_crew(run_id.as_str())?;
                self.terminate_crew(&run, CrewStatus::Failed, Some(error.to_string())).await
            }
        }
    }

    /// Resume (kill old agent + respawn) an entity.
    async fn resume_run(
        &self,
        run: &dyn RunLifecycle,
        agent_def: &oj_runbook::AgentDef,
        kill_agent: Option<String>,
        agent_name: &str,
        input: &HashMap<String, String>,
        resume: bool,
    ) -> Result<Vec<Event>, RuntimeError> {
        match run.owner_id() {
            OwnerId::Job(job_id) => {
                let job = self.require_job(job_id.as_str())?;
                let agent_id = kill_agent.map(AgentId::new);
                self.kill_and_resume(&job, agent_id, agent_name, input, resume).await
            }
            OwnerId::Crew(run_id) => {
                let run = self.require_crew(run_id.as_str())?;
                if kill_agent.is_some() {
                    self.capture_before_kill_crew(&run).await;
                }
                if let Some(aid) = kill_agent {
                    let _ = self
                        .executor
                        .execute(Effect::KillAgent { agent_id: AgentId::new(aid) })
                        .await;
                }
                let crew_id = CrewId::new(&run.id);
                self.spawn_standalone_agent(super::agent::SpawnAgentParams {
                    crew_id: &crew_id,
                    agent_def,
                    agent_name,
                    input,
                    cwd: &run.cwd,
                    project: &run.project,
                    resume,
                })
                .await
            }
        }
    }

    /// Run a gate command and handle pass/fail for any entity.
    async fn run_gate(
        &self,
        run: &dyn RunLifecycle,
        agent_def: &oj_runbook::AgentDef,
        command: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let owner = run.owner_id();
        let execution_dir = run.execution_dir().to_path_buf();

        // Interpolate command with entity vars
        let mut vars = crate::engine::vars::namespace_vars(run.vars());
        let (key, value) = run.owner_id_var();
        vars.insert(key.to_string(), value);
        vars.insert("name".to_string(), run.display_name().to_string());
        vars.insert("workspace".to_string(), execution_dir.display().to_string());
        let command = oj_runbook::interpolate_shell(command, &vars);

        self.log_entity_activity(
            run,
            &format!("gate (cwd: {}): {}", execution_dir.display(), command),
        );

        tracing::info!(
            entity_id = %run.log_id(),
            gate = %command,
            cwd = %execution_dir.display(),
            "running gate command"
        );

        match super::gate::run_gate_command(&command, &execution_dir).await {
            Ok(()) => {
                self.log_entity_activity(run, "gate passed, advancing");
                // Emit on_done notification on gate pass
                if let Some(effect) = crate::engine::lifecycle::notify_on_done(run, agent_def) {
                    self.executor.execute(effect).await?;
                }
                self.advance_run(run).await
            }
            Err(gate_error) => {
                self.log_entity_activity(run, &format!("gate failed: {}", gate_error));

                // Parse gate error for structured context
                let (exit_code, stderr) = super::gate::parse_gate_error(&gate_error);

                // Create decision with gate failure context
                let (decision_id, decision_event) = EscalationDecisionBuilder::new(
                    owner.clone(),
                    run.display_name().to_string(),
                    run.decision_agent_ref(),
                    EscalationTrigger::GateFailed { command: command.clone(), exit_code, stderr },
                )
                .project(run.project())
                .build();

                let mut effects = vec![Effect::Emit { event: decision_event }];

                // Entity-specific status changes and timer cancellations
                effects.extend(run.escalation_effects(&gate_error, Some(decision_id.as_str())));

                Ok(self.executor.execute_all(effects).await?)
            }
        }
    }

    async fn kill_and_resume(
        &self,
        job: &Job,
        kill_agent: Option<AgentId>,
        agent_name: &str,
        input: &HashMap<String, String>,
        resume: bool,
    ) -> Result<Vec<Event>, RuntimeError> {
        if let Some(aid) = kill_agent {
            self.capture_before_kill_job(job).await;
            self.executor.execute(Effect::KillAgent { agent_id: aid }).await?;
        }
        let job_id = JobId::new(&job.id);
        self.spawn_agent_with_resume(&job_id, agent_name, input, resume).await
    }

    /// Log to job activity log if entity is a Job (no-op for crew).
    fn log_entity_activity(&self, run: &dyn RunLifecycle, message: &str) {
        if let Some(step) = run.step() {
            self.logger.append(run.log_id(), step, message);
        }
    }
}

/// Get the agent_id for the current step from a job's step history.
pub(super) fn step_agent_id(job: &Job) -> Option<&str> {
    job.step_history.iter().rfind(|r| r.name == job.step).and_then(|r| r.agent_id.as_deref())
}
