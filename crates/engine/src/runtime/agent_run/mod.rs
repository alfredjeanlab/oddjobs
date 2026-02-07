// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Standalone agent run lifecycle handling

mod effects;
mod lifecycle;
mod spawn;

use crate::error::RuntimeError;
use crate::runtime::Runtime;
use oj_adapters::agent::find_session_log;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{
    AgentId, AgentRun, AgentRunId, AgentRunStatus, Clock, Effect, Event, OwnerId, SessionId,
    TimerId, WorkspaceId,
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
    /// Terminate a standalone agent run with the given status.
    ///
    /// Emits a status change event, cancels the liveness timer, kills the
    /// tmux session, and deletes owned workspaces.
    async fn terminate_agent_run(
        &self,
        agent_run: &AgentRun,
        status: AgentRunStatus,
        reason: Option<String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let agent_run_id = AgentRunId::new(&agent_run.id);
        let events = vec![
            Effect::Emit {
                event: Event::AgentRunStatusChanged {
                    id: agent_run_id.clone(),
                    status,
                    reason,
                },
            },
            Effect::CancelTimer {
                id: TimerId::liveness_agent_run(&agent_run_id),
            },
        ];
        let result = self.executor.execute_all(events).await?;
        self.cleanup_standalone_agent_session(agent_run).await?;
        self.cleanup_agent_run_workspaces(&agent_run_id).await?;
        Ok(result)
    }

    /// Delete workspaces owned by a standalone agent run.
    async fn cleanup_agent_run_workspaces(
        &self,
        agent_run_id: &AgentRunId,
    ) -> Result<(), RuntimeError> {
        let ar_owner = OwnerId::agent_run(agent_run_id.clone());
        let ws_ids: Vec<WorkspaceId> = self.lock_state(|s| {
            s.workspaces
                .values()
                .filter(|ws| ws.owner.as_ref() == Some(&ar_owner))
                .map(|ws| WorkspaceId::new(&ws.id))
                .collect()
        });
        for ws_id in ws_ids {
            let _ = self
                .executor
                .execute(Effect::DeleteWorkspace {
                    workspace_id: ws_id,
                })
                .await;
        }
        Ok(())
    }

    /// Kill the standalone agent's tmux session and clean up mappings.
    async fn cleanup_standalone_agent_session(
        &self,
        agent_run: &AgentRun,
    ) -> Result<(), RuntimeError> {
        if let Some(ref aid) = agent_run.agent_id {
            self.deregister_agent(&AgentId::new(aid));
        }
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
}
