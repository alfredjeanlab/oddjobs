// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent and workspace methods for DaemonClient.

use std::path::PathBuf;

use oj_wire::{Query, Request, Response};

use super::super::{ClientError, DaemonClient};

impl DaemonClient {
    /// Query for a specific agent by ID (or prefix)
    pub async fn get_agent(
        &self,
        agent_id: &str,
    ) -> Result<Option<oj_wire::AgentDetail>, ClientError> {
        let request = Request::Query { query: Query::GetAgent { agent_id: agent_id.to_string() } };
        match self.send(&request).await? {
            Response::Agent { agent } => Ok(agent.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Query for agents across all jobs
    pub async fn list_agents(
        &self,
        job_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<oj_wire::AgentSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListAgents {
                job_id: job_id.map(|s| s.to_string()),
                status: status.map(|s| s.to_string()),
            },
        };
        match self.send(&query).await? {
            Response::Agents { agents } => Ok(agents),
            other => Self::reject(other),
        }
    }

    /// Send a message to a running agent
    pub async fn agent_send(&self, agent_id: &str, message: &str) -> Result<(), ClientError> {
        let request = Request::AgentSend { id: agent_id.to_string(), message: message.to_string() };
        self.send_simple(&request).await
    }

    /// Kill an agent's session (triggers on_dead lifecycle handling)
    pub async fn agent_kill(&self, agent_id: &str) -> Result<(), ClientError> {
        let request = Request::AgentKill { id: agent_id.to_string() };
        self.send_simple(&request).await
    }

    /// Get agent logs
    pub async fn get_agent_logs(
        &self,
        id: &str,
        step: Option<&str>,
        lines: usize,
        offset: u64,
    ) -> Result<(PathBuf, String, Vec<String>, u64), ClientError> {
        let request = Request::Query {
            query: Query::GetAgentLogs {
                id: id.to_string(),
                step: step.map(|s| s.to_string()),
                lines,
                offset,
            },
        };
        match self.send(&request).await? {
            Response::AgentLogs { log_path, content, steps, offset } => {
                Ok((log_path, content, steps, offset))
            }
            other => Self::reject(other),
        }
    }

    /// Resume an agent (re-spawn with --resume to preserve conversation)
    pub async fn agent_resume(
        &self,
        agent_id: &str,
        kill: bool,
        all: bool,
    ) -> Result<(Vec<String>, Vec<(String, String)>), ClientError> {
        let request = Request::AgentResume { id: agent_id.to_string(), kill, all };
        match self.send(&request).await? {
            Response::AgentResumed { resumed, skipped } => Ok((resumed, skipped)),
            other => Self::reject(other),
        }
    }

    /// Prune agent logs from terminal jobs
    pub async fn agent_prune(
        &self,
        all: bool,
        dry_run: bool,
    ) -> Result<(Vec<oj_wire::AgentEntry>, usize), ClientError> {
        match self.send(&Request::AgentPrune { all, dry_run }).await? {
            Response::AgentsPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    // -- Workspace queries --

    /// Query for workspaces
    pub async fn list_workspaces(&self) -> Result<Vec<oj_wire::WorkspaceSummary>, ClientError> {
        let query = Request::Query { query: Query::ListWorkspaces };
        match self.send(&query).await? {
            Response::Workspaces { workspaces } => Ok(workspaces),
            other => Self::reject(other),
        }
    }

    /// Query for a specific workspace
    pub async fn get_workspace(
        &self,
        id: &str,
    ) -> Result<Option<oj_wire::WorkspaceDetail>, ClientError> {
        let request = Request::Query { query: Query::GetWorkspace { id: id.to_string() } };
        match self.send(&request).await? {
            Response::Workspace { workspace } => Ok(workspace.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Delete workspaces by sending one of the drop request variants.
    pub async fn workspace_drop(
        &self,
        request: Request,
    ) -> Result<Vec<oj_wire::WorkspaceEntry>, ClientError> {
        match self.send(&request).await? {
            Response::WorkspacesDropped { dropped } => Ok(dropped),
            other => Self::reject(other),
        }
    }

    /// Prune old workspaces from terminal jobs
    pub async fn workspace_prune(
        &self,
        all: bool,
        dry_run: bool,
        project: Option<&str>,
    ) -> Result<(Vec<oj_wire::WorkspaceEntry>, usize), ClientError> {
        let req = Request::WorkspacePrune { all, dry_run, project: project.map(String::from) };
        match self.send(&req).await? {
            Response::WorkspacesPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }
}
