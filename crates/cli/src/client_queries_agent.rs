// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent, session, and workspace methods for DaemonClient.

use std::path::PathBuf;

use oj_daemon::{Query, Request, Response};

use super::super::{AgentSignalResponse, ClientError, DaemonClient};

impl DaemonClient {
    /// Query for a specific agent by ID (or prefix)
    pub async fn get_agent(
        &self,
        agent_id: &str,
    ) -> Result<Option<oj_daemon::AgentDetail>, ClientError> {
        let request = Request::Query {
            query: Query::GetAgent {
                agent_id: agent_id.to_string(),
            },
        };
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
    ) -> Result<Vec<oj_daemon::AgentSummary>, ClientError> {
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

    /// Query for sessions
    pub async fn list_sessions(&self) -> Result<Vec<oj_daemon::SessionSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListSessions,
        };
        match self.send(&query).await? {
            Response::Sessions { sessions } => Ok(sessions),
            other => Self::reject(other),
        }
    }

    /// Send a message to a running agent
    pub async fn agent_send(&self, agent_id: &str, message: &str) -> Result<(), ClientError> {
        let request = Request::AgentSend {
            agent_id: agent_id.to_string(),
            message: message.to_string(),
        };
        self.send_simple(&request).await
    }

    /// Kill an agent's session (triggers on_dead lifecycle handling)
    pub async fn agent_kill(&self, agent_id: &str) -> Result<(), ClientError> {
        let request = Request::AgentKill {
            agent_id: agent_id.to_string(),
        };
        self.send_simple(&request).await
    }

    /// Kill a session
    pub async fn session_kill(&self, id: &str) -> Result<(), ClientError> {
        let request = Request::SessionKill { id: id.to_string() };
        self.send_simple(&request).await
    }

    /// Send input to a session
    pub async fn session_send(&self, id: &str, input: &str) -> Result<(), ClientError> {
        let request = Request::SessionSend {
            id: id.to_string(),
            input: input.to_string(),
        };
        self.send_simple(&request).await
    }

    /// Query for a specific session by ID (or prefix)
    pub async fn get_session(
        &self,
        id: &str,
    ) -> Result<Option<oj_daemon::SessionSummary>, ClientError> {
        let request = Request::Query {
            query: Query::GetSession { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Session { session } => Ok(session.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Peek at a session's tmux pane output
    pub async fn peek_session(
        &self,
        session_id: &str,
        with_color: bool,
    ) -> Result<String, ClientError> {
        let request = Request::PeekSession {
            session_id: session_id.to_string(),
            with_color,
        };
        match self.send(&request).await? {
            Response::SessionPeek { output } => Ok(output),
            other => Self::reject(other),
        }
    }

    /// Get agent logs
    pub async fn get_agent_logs(
        &self,
        id: &str,
        step: Option<&str>,
        lines: usize,
    ) -> Result<(PathBuf, String, Vec<String>), ClientError> {
        let request = Request::Query {
            query: Query::GetAgentLogs {
                id: id.to_string(),
                step: step.map(|s| s.to_string()),
                lines,
            },
        };
        match self.send(&request).await? {
            Response::AgentLogs {
                log_path,
                content,
                steps,
            } => Ok((log_path, content, steps)),
            other => Self::reject(other),
        }
    }

    /// Query if an agent has signaled completion (for stop hook)
    pub async fn query_agent_signal(
        &self,
        agent_id: &str,
    ) -> Result<AgentSignalResponse, ClientError> {
        let request = Request::Query {
            query: Query::GetAgentSignal {
                agent_id: agent_id.to_string(),
            },
        };
        match self.send(&request).await? {
            Response::AgentSignal { signaled, .. } => Ok(AgentSignalResponse { signaled }),
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
        let request = Request::AgentResume {
            agent_id: agent_id.to_string(),
            kill,
            all,
        };
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
    ) -> Result<(Vec<oj_daemon::AgentEntry>, usize), ClientError> {
        match self.send(&Request::AgentPrune { all, dry_run }).await? {
            Response::AgentsPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    /// Prune orphaned sessions (from terminal or missing jobs)
    pub async fn session_prune(
        &self,
        all: bool,
        dry_run: bool,
        namespace: Option<String>,
    ) -> Result<SessionPruneResult, ClientError> {
        let request = Request::SessionPrune {
            all,
            dry_run,
            namespace,
        };
        match self.send(&request).await? {
            Response::SessionsPruned { pruned, skipped } => {
                Ok(SessionPruneResult { pruned, skipped })
            }
            other => Self::reject(other),
        }
    }

    // -- Workspace queries --

    /// Query for workspaces
    pub async fn list_workspaces(&self) -> Result<Vec<oj_daemon::WorkspaceSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListWorkspaces,
        };
        match self.send(&query).await? {
            Response::Workspaces { workspaces } => Ok(workspaces),
            other => Self::reject(other),
        }
    }

    /// Query for a specific workspace
    pub async fn get_workspace(
        &self,
        id: &str,
    ) -> Result<Option<oj_daemon::WorkspaceDetail>, ClientError> {
        let request = Request::Query {
            query: Query::GetWorkspace { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Workspace { workspace } => Ok(workspace.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Delete a specific workspace by ID
    pub async fn workspace_drop(
        &self,
        id: &str,
    ) -> Result<Vec<oj_daemon::WorkspaceEntry>, ClientError> {
        self.send_workspace_drop(Request::WorkspaceDrop { id: id.to_string() })
            .await
    }

    /// Delete all failed workspaces
    pub async fn workspace_drop_failed(
        &self,
    ) -> Result<Vec<oj_daemon::WorkspaceEntry>, ClientError> {
        self.send_workspace_drop(Request::WorkspaceDropFailed).await
    }

    /// Delete all workspaces
    pub async fn workspace_drop_all(&self) -> Result<Vec<oj_daemon::WorkspaceEntry>, ClientError> {
        self.send_workspace_drop(Request::WorkspaceDropAll).await
    }

    async fn send_workspace_drop(
        &self,
        request: Request,
    ) -> Result<Vec<oj_daemon::WorkspaceEntry>, ClientError> {
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
        namespace: Option<&str>,
    ) -> Result<(Vec<oj_daemon::WorkspaceEntry>, usize), ClientError> {
        let req = Request::WorkspacePrune {
            all,
            dry_run,
            namespace: namespace.map(String::from),
        };
        match self.send(&req).await? {
            Response::WorkspacesPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }
}

/// Result from session prune operation
pub struct SessionPruneResult {
    pub pruned: Vec<oj_daemon::SessionEntry>,
    pub skipped: usize,
}
