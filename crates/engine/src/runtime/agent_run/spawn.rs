// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Standalone agent spawning, resume, and recovery

use super::SpawnAgentParams;
use crate::error::RuntimeError;
use crate::runtime::Runtime;
use oj_adapters::agent::find_session_log;
use oj_adapters::{AgentAdapter, AgentReconnectConfig, NotifyAdapter, SessionAdapter};
use oj_core::{
    AgentId, AgentRunId, AgentRunStatus, Clock, Effect, Event, OwnerId, SessionId, TimerId,
};

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

        // Build a SpawnCtx for standalone agent
        let ctx = crate::spawn::SpawnCtx::from_agent_run(agent_run_id, agent_name, namespace);

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

        // Execute spawn effects (SpawnAgent fires a background task and returns immediately)
        let mut result_events = self.executor.execute_all(effects).await?;

        // Emit AgentRunStarted event if we have an agent_id
        // (records the agent_id immediately in state for queries)
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

        // on_start notification and liveness timer are set by
        // handle_session_created() when the background spawn completes.
        // Spawn failures arrive as AgentSpawnFailed events and are
        // handled by handle_agent_spawn_failed().

        Ok(result_events)
    }

    /// Handle resume for a standalone agent: nudge if alive, respawn if dead.
    pub(crate) async fn handle_agent_run_resume(
        &self,
        agent_run_id: &AgentRunId,
        message: Option<&str>,
        kill: bool,
    ) -> Result<Vec<Event>, RuntimeError> {
        let agent_run = self
            .lock_state(|s| s.agent_runs.get(agent_run_id.as_str()).cloned())
            .ok_or_else(|| {
                RuntimeError::InvalidRequest(format!("agent run not found: {}", agent_run_id))
            })?;

        let runbook = self.cached_runbook(&agent_run.runbook_hash)?;
        let agent_def = runbook
            .get_agent(&agent_run.agent_name)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_run.agent_name.clone()))?
            .clone();

        // Check if agent is alive
        let agent_id = agent_run.agent_id.as_ref().map(AgentId::new);
        let agent_state = match &agent_id {
            Some(id) => self.executor.get_agent_state(id).await.ok(),
            None => None,
        };
        let is_alive = matches!(
            agent_state,
            Some(oj_core::AgentState::Working) | Some(oj_core::AgentState::WaitingForInput)
        );

        // If alive and not killing, nudge the agent
        if !kill && is_alive {
            if let Some(id) = &agent_id {
                if let Some(msg) = message {
                    self.executor
                        .execute(Effect::SendToAgent {
                            agent_id: id.clone(),
                            input: msg.to_string(),
                        })
                        .await?;
                }

                // Reset status to Running
                self.executor
                    .execute(Effect::Emit {
                        event: Event::AgentRunStatusChanged {
                            id: agent_run_id.clone(),
                            status: AgentRunStatus::Running,
                            reason: Some("resumed".to_string()),
                        },
                    })
                    .await?;

                // Restart liveness timer
                self.executor
                    .execute(Effect::SetTimer {
                        id: TimerId::liveness_agent_run(agent_run_id),
                        duration: crate::spawn::LIVENESS_INTERVAL,
                    })
                    .await?;

                // Record nudge timestamp to suppress auto-resume from our own nudge text
                let now = self.clock().epoch_ms();
                self.lock_state_mut(|state| {
                    if let Some(ar) = state.agent_runs.get_mut(agent_run_id.as_str()) {
                        ar.last_nudge_at = Some(now);
                    }
                });

                tracing::info!(agent_run_id = %agent_run.id, "nudged standalone agent");
                return Ok(vec![]);
            }
        }

        // Agent dead OR --kill requested: recover using --resume
        // Kill old tmux session if it exists
        if let Some(ref session_id) = agent_run.session_id {
            let _ = self
                .executor
                .execute(Effect::KillSession {
                    session_id: SessionId::new(session_id),
                })
                .await;
        }

        // Find a valid session file to resume from
        let resume_session_id = agent_run
            .agent_id
            .as_ref()
            .and_then(|aid| find_session_log(&agent_run.cwd, aid).map(|_| aid.clone()));

        // Respawn agent with resume
        let mut input = agent_run.vars.clone();
        if let Some(msg) = message {
            input.insert("resume_message".to_string(), msg.to_string());
        }

        let result = self
            .spawn_standalone_agent(SpawnAgentParams {
                agent_run_id,
                agent_def: &agent_def,
                agent_name: &agent_run.agent_name,
                input: &input,
                cwd: &agent_run.cwd,
                namespace: &agent_run.namespace,
                resume_session_id: resume_session_id.as_deref(),
            })
            .await?;

        tracing::info!(
            agent_run_id = %agent_run.id,
            kill,
            resume = resume_session_id.is_some(),
            "resumed standalone agent with --resume"
        );
        Ok(result)
    }

    /// Reconnect monitoring for a standalone agent that survived a daemon restart.
    pub async fn recover_standalone_agent(
        &self,
        agent_run: &oj_core::AgentRun,
    ) -> Result<(), RuntimeError> {
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
            owner: OwnerId::agent_run(agent_run_id.clone()),
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
