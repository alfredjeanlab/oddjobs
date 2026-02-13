// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Crew lifecycle handling

use crate::adapters::AgentReconnectConfig;
use crate::engine::error::RuntimeError;
use crate::engine::runtime::Runtime;
use oj_core::{
    AgentId, Clock, Crew, CrewId, CrewStatus, Effect, Event, OwnerId, TimerId, WorkspaceId,
};
use oj_runbook::AgentDef;
use std::collections::HashMap;
use std::path::Path;

/// Parameters for spawning a standalone agent.
pub(crate) struct SpawnAgentParams<'a> {
    pub crew_id: &'a CrewId,
    pub agent_def: &'a AgentDef,
    pub agent_name: &'a str,
    pub input: &'a HashMap<String, String>,
    pub cwd: &'a Path,
    pub project: &'a str,
    pub resume: bool,
}

impl<C: Clock> Runtime<C> {
    /// Terminate a crew with the given status.
    ///
    /// Emits a status change event, cancels the liveness timer, kills the
    /// agent process, and deletes owned workspaces.
    pub(super) async fn terminate_crew(
        &self,
        crew: &Crew,
        status: CrewStatus,
        reason: Option<String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let crew_id = CrewId::new(&crew.id);
        let events = vec![
            Effect::Emit { event: Event::CrewUpdated { id: crew_id.clone(), status, reason } },
            Effect::CancelTimer { id: TimerId::liveness(&crew_id) },
        ];
        let result = self.executor.execute_all(events).await?;
        self.cleanup_standalone_agent_session(crew).await?;
        self.cleanup_crew_workspaces(&crew_id).await?;
        Ok(result)
    }

    /// Delete workspaces owned by a crew.
    async fn cleanup_crew_workspaces(&self, crew_id: &CrewId) -> Result<(), RuntimeError> {
        let ar_owner = OwnerId::crew(crew_id.clone());
        let ws_ids: Vec<WorkspaceId> = self.lock_state(|s| {
            s.workspaces
                .values()
                .filter(|ws| ws.owner == ar_owner)
                .map(|ws| WorkspaceId::new(&ws.id))
                .collect()
        });
        for ws_id in ws_ids {
            let _ = self.executor.execute(Effect::DeleteWorkspace { workspace_id: ws_id }).await;
        }
        Ok(())
    }

    /// Kill the standalone agent and clean up mappings.
    async fn cleanup_standalone_agent_session(&self, crew: &Crew) -> Result<(), RuntimeError> {
        // Capture terminal + session log before killing
        self.capture_before_kill_crew(crew).await;

        if let Some(ref aid) = crew.agent_id {
            self.agent_owners.lock().remove(&AgentId::new(aid));
            let _ = self.executor.execute(Effect::KillAgent { agent_id: AgentId::new(aid) }).await;
        }
        Ok(())
    }
}

impl<C: Clock> Runtime<C> {
    /// Spawn a standalone agent for a command run.
    ///
    /// Builds spawn effects using the agent definition, registers the agent→run
    /// mapping, and executes the effects. Returns events produced.
    pub(crate) async fn spawn_standalone_agent(
        &self,
        params: SpawnAgentParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let SpawnAgentParams { crew_id, agent_def, agent_name, input, cwd, project, resume } =
            params;

        // Build a SpawnCtx for standalone agent
        let ctx = crate::engine::spawn::SpawnCtx::from_crew(crew_id, agent_name, project);

        let effects = crate::engine::spawn::build_spawn_effects(
            agent_def,
            &ctx,
            agent_name,
            input,
            cwd,
            &self.state_dir,
            resume,
        )?;

        // Extract agent_id from SpawnAgent effect
        let agent_id = effects.iter().find_map(|e| match e {
            Effect::SpawnAgent { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        });

        // Register agent → crew mapping
        if let Some(ref aid) = agent_id {
            self.register_agent(aid.clone(), OwnerId::crew(crew_id.clone()));
        }

        // Execute spawn effects (SpawnAgent fires a background task and returns immediately)
        let mut result_events = self.executor.execute_all(effects).await?;

        // Emit CrewStarted event if we have an agent_id
        // (records the agent_id immediately in state for queries)
        if let Some(ref aid) = agent_id {
            let started_event = Event::CrewStarted { id: crew_id.clone(), agent_id: aid.clone() };
            if let Some(ev) = self.executor.execute(Effect::Emit { event: started_event }).await? {
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
    pub(crate) async fn handle_crew_resume(
        &self,
        crew_id: &CrewId,
        message: Option<&str>,
        kill: bool,
    ) -> Result<Vec<Event>, RuntimeError> {
        let crew = self
            .lock_state(|s| s.crew.get(crew_id.as_str()).cloned())
            .ok_or_else(|| RuntimeError::InvalidRequest(format!("crew not found: {}", crew_id)))?;

        let runbook = self.cached_runbook(&crew.runbook_hash)?;
        let agent_def = runbook
            .get_agent(&crew.agent_name)
            .ok_or_else(|| RuntimeError::AgentNotFound(crew.agent_name.clone()))?
            .clone();

        // Check if agent is alive
        let agent_id = crew.agent_id.as_ref().map(AgentId::new);
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
                        event: Event::CrewUpdated {
                            id: crew_id.clone(),
                            status: CrewStatus::Running,
                            reason: Some("resumed".to_string()),
                        },
                    })
                    .await?;

                // Restart liveness timer
                self.executor
                    .execute(Effect::SetTimer {
                        id: TimerId::liveness(crew_id),
                        duration: crate::engine::spawn::LIVENESS_INTERVAL,
                    })
                    .await?;

                // Record nudge timestamp to suppress auto-resume from our own nudge text
                let now = self.executor.clock().epoch_ms();
                self.lock_state_mut(|state| {
                    if let Some(run) = state.crew.get_mut(crew_id.as_str()) {
                        run.last_nudge_at = Some(now);
                    }
                });

                tracing::info!(crew_id = %crew.id, "nudged standalone agent");
                return Ok(vec![]);
            }
        }

        // Agent dead OR --kill requested: recover using --resume
        let resume = crew.agent_id.is_some();
        if let Some(ref aid) = crew.agent_id {
            let _ = self.executor.execute(Effect::KillAgent { agent_id: AgentId::new(aid) }).await;
        }

        // Respawn agent with resume (coop handles session discovery)
        let mut input = crew.vars.clone();
        if let Some(msg) = message {
            input.insert("resume_message".to_string(), msg.to_string());
        }

        let result = self
            .spawn_standalone_agent(SpawnAgentParams {
                crew_id,
                agent_def: &agent_def,
                agent_name: &crew.agent_name,
                input: &input,
                cwd: &crew.cwd,
                project: &crew.project,
                resume,
            })
            .await?;

        tracing::info!(crew_id = %crew.id, kill, resume, "resumed standalone agent with --resume");
        Ok(result)
    }

    /// Reconnect monitoring for a standalone agent that survived a daemon restart.
    pub async fn recover_standalone_agent(&self, crew: &oj_core::Crew) -> Result<(), RuntimeError> {
        let agent_id_str = crew.agent_id.as_ref().ok_or_else(|| {
            RuntimeError::InvalidRequest(format!("crew {} has no agent_id", crew.id))
        })?;
        let agent_id = AgentId::new(agent_id_str);
        let crew_id = CrewId::new(&crew.id);

        // Look up persisted runtime and auth token from agent records
        let (runtime_hint, auth_token) = self.lock_state(|s| {
            s.agents
                .get(agent_id_str)
                .map(|r| (r.runtime, r.auth_token.clone()))
                .unwrap_or_default()
        });

        // Register agent → crew mapping
        self.register_agent(agent_id.clone(), OwnerId::crew(crew_id.clone()));

        let config = AgentReconnectConfig {
            agent_id,
            owner: OwnerId::crew(crew_id.clone()),
            runtime_hint,
            auth_token,
        };
        self.executor.reconnect_agent(config).await?;

        // Restore liveness timer
        self.executor
            .execute(Effect::SetTimer {
                id: TimerId::liveness(&crew_id),
                duration: crate::engine::spawn::LIVENESS_INTERVAL,
            })
            .await?;

        Ok(())
    }
}
