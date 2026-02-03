// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Query and command methods for DaemonClient.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oj_daemon::{Query, Request, Response};

use super::{AgentSignalResponse, CancelResult, ClientError, DaemonClient};

/// Result from running a command — either a pipeline or a standalone agent
pub enum RunCommandResult {
    Pipeline {
        pipeline_id: String,
        pipeline_name: String,
    },
    AgentRun {
        agent_run_id: String,
        agent_name: String,
    },
}

impl DaemonClient {
    /// Query for pipelines
    pub async fn list_pipelines(&self) -> Result<Vec<oj_daemon::PipelineSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListPipelines,
        };
        match self.send(&query).await? {
            Response::Pipelines { pipelines } => Ok(pipelines),
            other => Self::reject(other),
        }
    }

    /// Query for a specific pipeline
    pub async fn get_pipeline(
        &self,
        id: &str,
    ) -> Result<Option<oj_daemon::PipelineDetail>, ClientError> {
        let request = Request::Query {
            query: Query::GetPipeline { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Pipeline { pipeline } => Ok(pipeline.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Get daemon status
    pub async fn status(&self) -> Result<(u64, usize, usize, usize), ClientError> {
        match self.send(&Request::Status).await? {
            Response::Status {
                uptime_secs,
                pipelines_active,
                sessions_active,
                orphan_count,
            } => Ok((uptime_secs, pipelines_active, sessions_active, orphan_count)),
            other => Self::reject(other),
        }
    }

    /// Request daemon shutdown
    pub async fn shutdown(&self, kill: bool) -> Result<(), ClientError> {
        match self.send(&Request::Shutdown { kill }).await? {
            Response::Ok | Response::ShuttingDown => Ok(()),
            other => Self::reject(other),
        }
    }

    /// Get daemon version via Hello handshake
    pub async fn hello(&self) -> Result<String, ClientError> {
        let request = Request::Hello {
            version: concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH")).to_string(),
        };
        match self.send(&request).await? {
            Response::Hello { version } => Ok(version),
            other => Self::reject(other),
        }
    }

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

    /// Query for agents across all pipelines
    pub async fn list_agents(
        &self,
        pipeline_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<oj_daemon::AgentSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListAgents {
                pipeline_id: pipeline_id.map(|s| s.to_string()),
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

    /// Resume monitoring for an escalated pipeline
    pub async fn pipeline_resume(
        &self,
        id: &str,
        message: Option<&str>,
        vars: &HashMap<String, String>,
    ) -> Result<(), ClientError> {
        let request = Request::PipelineResume {
            id: id.to_string(),
            message: message.map(String::from),
            vars: vars.clone(),
        };
        self.send_simple(&request).await
    }

    /// Cancel one or more pipelines by ID
    pub async fn pipeline_cancel(&self, ids: &[String]) -> Result<CancelResult, ClientError> {
        let request = Request::PipelineCancel { ids: ids.to_vec() };
        match self.send(&request).await? {
            Response::PipelinesCancelled {
                cancelled,
                already_terminal,
                not_found,
            } => Ok(CancelResult {
                cancelled,
                already_terminal,
                not_found,
            }),
            other => Self::reject(other),
        }
    }

    /// Query for workspaces
    pub async fn list_workspaces(&self) -> Result<Vec<oj_daemon::WorkspaceSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListWorkspaces,
        };
        match self.send(&query).await? {
            Response::Workspaces { workspaces } => Ok(workspaces),
            Response::Error { message } => Err(ClientError::Rejected(message)),
            _ => Err(ClientError::UnexpectedResponse),
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
            Response::Error { message } => Err(ClientError::Rejected(message)),
            _ => Err(ClientError::UnexpectedResponse),
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

    /// Get pipeline logs
    pub async fn get_pipeline_logs(
        &self,
        id: &str,
        lines: usize,
    ) -> Result<(PathBuf, String), ClientError> {
        let request = Request::Query {
            query: Query::GetPipelineLogs {
                id: id.to_string(),
                lines,
            },
        };
        match self.send(&request).await? {
            Response::PipelineLogs { log_path, content } => Ok((log_path, content)),
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

    /// Run a command from the project runbook
    pub async fn run_command(
        &self,
        project_root: &Path,
        invoke_dir: &Path,
        namespace: &str,
        command: &str,
        args: &[String],
        named_args: &HashMap<String, String>,
    ) -> Result<RunCommandResult, ClientError> {
        let request = Request::RunCommand {
            project_root: project_root.to_path_buf(),
            invoke_dir: invoke_dir.to_path_buf(),
            namespace: namespace.to_string(),
            command: command.to_string(),
            args: args.to_vec(),
            named_args: named_args.clone(),
        };
        match self.send(&request).await? {
            Response::CommandStarted {
                pipeline_id,
                pipeline_name,
            } => Ok(RunCommandResult::Pipeline {
                pipeline_id,
                pipeline_name,
            }),
            Response::AgentRunStarted {
                agent_run_id,
                agent_name,
            } => Ok(RunCommandResult::AgentRun {
                agent_run_id,
                agent_name,
            }),
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

    /// Prune old terminal pipelines and their log files
    pub async fn pipeline_prune(
        &self,
        all: bool,
        failed: bool,
        orphans: bool,
        dry_run: bool,
        namespace: Option<&str>,
    ) -> Result<(Vec<oj_daemon::PipelineEntry>, usize), ClientError> {
        let req = Request::PipelinePrune {
            all,
            failed,
            orphans,
            dry_run,
            namespace: namespace.map(String::from),
        };
        match self.send(&req).await? {
            Response::PipelinesPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    /// Prune agent logs from terminal pipelines
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

    /// Prune old workspaces from terminal pipelines
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

    /// Prune stopped crons from daemon state
    pub async fn cron_prune(
        &self,
        all: bool,
        dry_run: bool,
    ) -> Result<(Vec<oj_daemon::CronEntry>, usize), ClientError> {
        match self.send(&Request::CronPrune { all, dry_run }).await? {
            Response::CronsPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    /// Prune stopped workers from daemon state
    pub async fn worker_prune(
        &self,
        all: bool,
        dry_run: bool,
        namespace: Option<&str>,
    ) -> Result<(Vec<oj_daemon::WorkerEntry>, usize), ClientError> {
        match self
            .send(&Request::WorkerPrune {
                all,
                dry_run,
                namespace: namespace.map(String::from),
            })
            .await?
        {
            Response::WorkersPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    /// Get worker activity logs
    pub async fn get_worker_logs(
        &self,
        name: &str,
        namespace: &str,
        lines: usize,
        project_root: Option<&Path>,
    ) -> Result<(PathBuf, String), ClientError> {
        let request = Request::Query {
            query: Query::GetWorkerLogs {
                name: name.to_string(),
                namespace: namespace.to_string(),
                lines,
                project_root: project_root.map(|p| p.to_path_buf()),
            },
        };
        match self.send(&request).await? {
            Response::WorkerLogs { log_path, content } => Ok((log_path, content)),
            other => Self::reject(other),
        }
    }

    /// Get cron logs
    pub async fn get_cron_logs(
        &self,
        name: &str,
        lines: usize,
        project_root: Option<&Path>,
    ) -> Result<(PathBuf, String), ClientError> {
        let request = Request::Query {
            query: Query::GetCronLogs {
                name: name.to_string(),
                lines,
                project_root: project_root.map(|p| p.to_path_buf()),
            },
        };
        match self.send(&request).await? {
            Response::CronLogs { log_path, content } => Ok((log_path, content)),
            other => Self::reject(other),
        }
    }

    /// Get cross-project status overview
    pub async fn status_overview(
        &self,
    ) -> Result<(u64, Vec<oj_daemon::NamespaceStatus>), ClientError> {
        let query = Request::Query {
            query: Query::StatusOverview,
        };
        match self.send(&query).await? {
            Response::StatusOverview {
                uptime_secs,
                namespaces,
            } => Ok((uptime_secs, namespaces)),
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

    /// List orphaned pipelines detected at startup
    pub async fn list_orphans(&self) -> Result<Vec<oj_daemon::OrphanSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListOrphans,
        };
        match self.send(&request).await? {
            Response::Orphans { orphans } => Ok(orphans),
            other => Self::reject(other),
        }
    }

    /// List all projects with active work
    pub async fn list_projects(&self) -> Result<Vec<oj_daemon::ProjectSummary>, ClientError> {
        let req = Request::Query {
            query: Query::ListProjects,
        };
        match self.send(&req).await? {
            Response::Projects { projects } => Ok(projects),
            other => Self::reject(other),
        }
    }

    /// Get queue activity logs
    pub async fn get_queue_logs(
        &self,
        queue_name: &str,
        namespace: &str,
        lines: usize,
    ) -> Result<(PathBuf, String), ClientError> {
        let request = Request::Query {
            query: Query::GetQueueLogs {
                queue_name: queue_name.to_string(),
                namespace: namespace.to_string(),
                lines,
            },
        };
        match self.send(&request).await? {
            Response::QueueLogs { log_path, content } => Ok((log_path, content)),
            other => Self::reject(other),
        }
    }

    /// Dismiss an orphaned pipeline by deleting its breadcrumb
    pub async fn dismiss_orphan(&self, id: &str) -> Result<(), ClientError> {
        let request = Request::Query {
            query: Query::DismissOrphan { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Ok => Ok(()),
            other => Self::reject(other),
        }
    }
}
