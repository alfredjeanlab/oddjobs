// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Query and command methods for DaemonClient.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oj_daemon::{Query, Request, Response};

use super::{AgentSignalResponse, CancelResult, ClientError, DaemonClient};

/// Result from running a command — either a job or a standalone agent
pub enum RunCommandResult {
    Job {
        job_id: String,
        job_name: String,
    },
    AgentRun {
        agent_run_id: String,
        agent_name: String,
    },
}

impl DaemonClient {
    /// Query for jobs
    pub async fn list_jobs(&self) -> Result<Vec<oj_daemon::JobSummary>, ClientError> {
        let query = Request::Query {
            query: Query::ListJobs,
        };
        match self.send(&query).await? {
            Response::Jobs { jobs } => Ok(jobs),
            other => Self::reject(other),
        }
    }

    /// Query for a specific job
    pub async fn get_job(&self, id: &str) -> Result<Option<oj_daemon::JobDetail>, ClientError> {
        let request = Request::Query {
            query: Query::GetJob { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Job { job } => Ok(job.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Get daemon status
    pub async fn status(&self) -> Result<(u64, usize, usize, usize), ClientError> {
        match self.send(&Request::Status).await? {
            Response::Status {
                uptime_secs,
                jobs_active,
                sessions_active,
                orphan_count,
            } => Ok((uptime_secs, jobs_active, sessions_active, orphan_count)),
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

    /// Resume monitoring for an escalated job
    pub async fn job_resume(
        &self,
        id: &str,
        message: Option<&str>,
        vars: &HashMap<String, String>,
        kill: bool,
    ) -> Result<(), ClientError> {
        let request = Request::JobResume {
            id: id.to_string(),
            message: message.map(String::from),
            vars: vars.clone(),
            kill,
            all: false,
        };
        self.send_simple(&request).await
    }

    /// Resume all resumable jobs
    pub async fn job_resume_all(
        &self,
        kill: bool,
    ) -> Result<(Vec<String>, Vec<(String, String)>), ClientError> {
        let request = Request::JobResumeAll { kill };
        match self.send(&request).await? {
            Response::JobsResumed { resumed, skipped } => Ok((resumed, skipped)),
            other => Self::reject(other),
        }
    }

    /// Cancel one or more jobs by ID
    pub async fn job_cancel(&self, ids: &[String]) -> Result<CancelResult, ClientError> {
        let request = Request::JobCancel { ids: ids.to_vec() };
        match self.send(&request).await? {
            Response::JobsCancelled {
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

    /// Get job logs
    pub async fn get_job_logs(
        &self,
        id: &str,
        lines: usize,
    ) -> Result<(PathBuf, String), ClientError> {
        let request = Request::Query {
            query: Query::GetJobLogs {
                id: id.to_string(),
                lines,
            },
        };
        match self.send(&request).await? {
            Response::JobLogs { log_path, content } => Ok((log_path, content)),
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
            Response::CommandStarted { job_id, job_name } => {
                Ok(RunCommandResult::Job { job_id, job_name })
            }
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

    /// Prune old terminal jobs and their log files
    pub async fn job_prune(
        &self,
        all: bool,
        failed: bool,
        orphans: bool,
        dry_run: bool,
        namespace: Option<&str>,
    ) -> Result<(Vec<oj_daemon::JobEntry>, usize), ClientError> {
        let req = Request::JobPrune {
            all,
            failed,
            orphans,
            dry_run,
            namespace: namespace.map(String::from),
        };
        match self.send(&req).await? {
            Response::JobsPruned { pruned, skipped } => Ok((pruned, skipped)),
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
        namespace: &str,
        lines: usize,
        project_root: Option<&Path>,
    ) -> Result<(PathBuf, String), ClientError> {
        let request = Request::Query {
            query: Query::GetCronLogs {
                name: name.to_string(),
                namespace: namespace.to_string(),
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

    /// List orphaned jobs detected at startup
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

    /// Prune completed/dead items from a queue
    pub async fn queue_prune(
        &self,
        project_root: &Path,
        namespace: &str,
        queue_name: &str,
        all: bool,
        dry_run: bool,
    ) -> Result<(Vec<oj_daemon::QueueItemEntry>, usize), ClientError> {
        let req = Request::QueuePrune {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            queue_name: queue_name.to_string(),
            all,
            dry_run,
        };
        match self.send(&req).await? {
            Response::QueuesPruned { pruned, skipped } => Ok((pruned, skipped)),
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

    /// Dismiss an orphaned job by deleting its breadcrumb
    pub async fn dismiss_orphan(&self, id: &str) -> Result<(), ClientError> {
        let request = Request::Query {
            query: Query::DismissOrphan { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Ok => Ok(()),
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

    // -- Worker commands --

    /// Start a worker
    pub async fn worker_start(
        &self,
        project_root: &Path,
        namespace: &str,
        worker_name: &str,
        all: bool,
    ) -> Result<WorkerStartResult, ClientError> {
        let request = Request::WorkerStart {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            worker_name: worker_name.to_string(),
            all,
        };
        match self.send(&request).await? {
            Response::WorkerStarted { worker_name } => {
                Ok(WorkerStartResult::Single { worker_name })
            }
            Response::WorkersStarted { started, skipped } => {
                Ok(WorkerStartResult::Multiple { started, skipped })
            }
            other => Self::reject(other),
        }
    }

    /// Stop a worker
    pub async fn worker_stop(
        &self,
        name: &str,
        namespace: &str,
        project_root: Option<&Path>,
    ) -> Result<(), ClientError> {
        let request = Request::WorkerStop {
            worker_name: name.to_string(),
            namespace: namespace.to_string(),
            project_root: project_root.map(|p| p.to_path_buf()),
        };
        self.send_simple(&request).await
    }

    /// Restart a worker
    pub async fn worker_restart(
        &self,
        project_root: &Path,
        namespace: &str,
        name: &str,
    ) -> Result<String, ClientError> {
        let request = Request::WorkerRestart {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            worker_name: name.to_string(),
        };
        match self.send(&request).await? {
            Response::WorkerStarted { worker_name } => Ok(worker_name),
            other => Self::reject(other),
        }
    }

    /// Resize a worker's concurrency
    pub async fn worker_resize(
        &self,
        name: &str,
        namespace: &str,
        concurrency: u32,
    ) -> Result<(String, u32, u32), ClientError> {
        let request = Request::WorkerResize {
            worker_name: name.to_string(),
            namespace: namespace.to_string(),
            concurrency,
        };
        match self.send(&request).await? {
            Response::WorkerResized {
                worker_name,
                old_concurrency,
                new_concurrency,
            } => Ok((worker_name, old_concurrency, new_concurrency)),
            other => Self::reject(other),
        }
    }

    /// List all workers
    pub async fn list_workers(&self) -> Result<Vec<oj_daemon::WorkerSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListWorkers,
        };
        match self.send(&request).await? {
            Response::Workers { workers } => Ok(workers),
            other => Self::reject(other),
        }
    }

    // -- Cron commands --

    /// Start a cron
    pub async fn cron_start(
        &self,
        project_root: &Path,
        namespace: &str,
        cron_name: &str,
        all: bool,
    ) -> Result<CronStartResult, ClientError> {
        let request = Request::CronStart {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            cron_name: cron_name.to_string(),
            all,
        };
        match self.send(&request).await? {
            Response::CronStarted { cron_name } => Ok(CronStartResult::Single { cron_name }),
            Response::CronsStarted { started, skipped } => {
                Ok(CronStartResult::Multiple { started, skipped })
            }
            other => Self::reject(other),
        }
    }

    /// Stop a cron
    pub async fn cron_stop(
        &self,
        name: &str,
        namespace: &str,
        project_root: Option<&Path>,
    ) -> Result<(), ClientError> {
        let request = Request::CronStop {
            cron_name: name.to_string(),
            namespace: namespace.to_string(),
            project_root: project_root.map(|p| p.to_path_buf()),
        };
        self.send_simple(&request).await
    }

    /// Restart a cron
    pub async fn cron_restart(
        &self,
        project_root: &Path,
        namespace: &str,
        name: &str,
    ) -> Result<String, ClientError> {
        let request = Request::CronRestart {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            cron_name: name.to_string(),
        };
        match self.send(&request).await? {
            Response::CronStarted { cron_name } => Ok(cron_name),
            other => Self::reject(other),
        }
    }

    /// Run a cron's job once immediately
    pub async fn cron_once(
        &self,
        project_root: &Path,
        namespace: &str,
        name: &str,
    ) -> Result<(String, String), ClientError> {
        let request = Request::CronOnce {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            cron_name: name.to_string(),
        };
        match self.send(&request).await? {
            Response::CommandStarted { job_id, job_name } => Ok((job_id, job_name)),
            other => Self::reject(other),
        }
    }

    /// List all crons
    pub async fn list_crons(&self) -> Result<Vec<oj_daemon::protocol::CronSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListCrons,
        };
        match self.send(&request).await? {
            Response::Crons { crons } => Ok(crons),
            other => Self::reject(other),
        }
    }

    // -- Queue commands --

    /// Push an item to a queue
    pub async fn queue_push(
        &self,
        project_root: &Path,
        namespace: &str,
        queue_name: &str,
        data: serde_json::Value,
    ) -> Result<QueuePushResult, ClientError> {
        let request = Request::QueuePush {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            queue_name: queue_name.to_string(),
            data,
        };
        match self.send(&request).await? {
            Response::QueuePushed {
                queue_name,
                item_id,
            } => Ok(QueuePushResult::Pushed {
                queue_name,
                item_id,
            }),
            Response::Ok => Ok(QueuePushResult::Refreshed),
            other => Self::reject(other),
        }
    }

    /// Drop an item from a queue
    pub async fn queue_drop(
        &self,
        project_root: &Path,
        namespace: &str,
        queue_name: &str,
        item_id: &str,
    ) -> Result<(String, String), ClientError> {
        let request = Request::QueueDrop {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            queue_name: queue_name.to_string(),
            item_id: item_id.to_string(),
        };
        match self.send(&request).await? {
            Response::QueueDropped {
                queue_name,
                item_id,
            } => Ok((queue_name, item_id)),
            other => Self::reject(other),
        }
    }

    /// Retry dead or failed queue items
    pub async fn queue_retry(
        &self,
        project_root: &Path,
        namespace: &str,
        queue_name: &str,
        item_ids: Vec<String>,
        all_dead: bool,
        status: Option<String>,
    ) -> Result<QueueRetryResult, ClientError> {
        let request = Request::QueueRetry {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            queue_name: queue_name.to_string(),
            item_ids,
            all_dead,
            status,
        };
        match self.send(&request).await? {
            Response::QueueRetried {
                queue_name,
                item_id,
            } => Ok(QueueRetryResult::Single {
                queue_name,
                item_id,
            }),
            Response::QueueItemsRetried {
                queue_name,
                item_ids,
                already_retried,
                not_found,
            } => Ok(QueueRetryResult::Bulk {
                queue_name,
                item_ids,
                already_retried,
                not_found,
            }),
            other => Self::reject(other),
        }
    }

    /// Force-fail an active queue item
    pub async fn queue_fail(
        &self,
        project_root: &Path,
        namespace: &str,
        queue_name: &str,
        item_id: &str,
    ) -> Result<(String, String), ClientError> {
        let request = Request::QueueFail {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            queue_name: queue_name.to_string(),
            item_id: item_id.to_string(),
        };
        match self.send(&request).await? {
            Response::QueueFailed {
                queue_name,
                item_id,
            } => Ok((queue_name, item_id)),
            other => Self::reject(other),
        }
    }

    /// Force-complete an active queue item
    pub async fn queue_done(
        &self,
        project_root: &Path,
        namespace: &str,
        queue_name: &str,
        item_id: &str,
    ) -> Result<(String, String), ClientError> {
        let request = Request::QueueDone {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            queue_name: queue_name.to_string(),
            item_id: item_id.to_string(),
        };
        match self.send(&request).await? {
            Response::QueueCompleted {
                queue_name,
                item_id,
            } => Ok((queue_name, item_id)),
            other => Self::reject(other),
        }
    }

    /// Drain all pending items from a queue
    pub async fn queue_drain(
        &self,
        project_root: &Path,
        namespace: &str,
        queue_name: &str,
    ) -> Result<(String, Vec<oj_daemon::QueueItemSummary>), ClientError> {
        let request = Request::QueueDrain {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            queue_name: queue_name.to_string(),
        };
        match self.send(&request).await? {
            Response::QueueDrained { queue_name, items } => Ok((queue_name, items)),
            other => Self::reject(other),
        }
    }

    /// List all queues in a project
    pub async fn list_queues(
        &self,
        project_root: &Path,
        namespace: &str,
    ) -> Result<Vec<oj_daemon::QueueSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListQueues {
                project_root: project_root.to_path_buf(),
                namespace: namespace.to_string(),
            },
        };
        match self.send(&request).await? {
            Response::Queues { queues } => Ok(queues),
            other => Self::reject(other),
        }
    }

    /// List items in a specific queue
    pub async fn list_queue_items(
        &self,
        queue_name: &str,
        namespace: &str,
        project_root: Option<&Path>,
    ) -> Result<Vec<oj_daemon::QueueItemSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListQueueItems {
                queue_name: queue_name.to_string(),
                namespace: namespace.to_string(),
                project_root: project_root.map(|p| p.to_path_buf()),
            },
        };
        match self.send(&request).await? {
            Response::QueueItems { items } => Ok(items),
            other => Self::reject(other),
        }
    }

    // -- Decision commands --

    /// List pending decisions
    pub async fn list_decisions(
        &self,
        namespace: &str,
    ) -> Result<Vec<oj_daemon::protocol::DecisionSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListDecisions {
                namespace: namespace.to_string(),
            },
        };
        match self.send(&request).await? {
            Response::Decisions { decisions } => Ok(decisions),
            other => Self::reject(other),
        }
    }

    /// Get a single decision by ID
    pub async fn get_decision(
        &self,
        id: &str,
    ) -> Result<Option<oj_daemon::protocol::DecisionDetail>, ClientError> {
        let request = Request::Query {
            query: Query::GetDecision { id: id.to_string() },
        };
        match self.send(&request).await? {
            Response::Decision { decision } => Ok(decision.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Resolve a pending decision
    pub async fn decision_resolve(
        &self,
        id: &str,
        chosen: Option<usize>,
        message: Option<String>,
    ) -> Result<String, ClientError> {
        let request = Request::DecisionResolve {
            id: id.to_string(),
            chosen,
            message,
        };
        match self.send(&request).await? {
            Response::DecisionResolved { id } => Ok(id),
            other => Self::reject(other),
        }
    }
}

/// Result from session prune operation
pub struct SessionPruneResult {
    pub pruned: Vec<oj_daemon::SessionEntry>,
    pub skipped: usize,
}

/// Result from worker start operation
pub enum WorkerStartResult {
    Single {
        worker_name: String,
    },
    Multiple {
        started: Vec<String>,
        skipped: Vec<(String, String)>,
    },
}

/// Result from cron start operation
pub enum CronStartResult {
    Single {
        cron_name: String,
    },
    Multiple {
        started: Vec<String>,
        skipped: Vec<(String, String)>,
    },
}

/// Result from queue push operation
pub enum QueuePushResult {
    Pushed { queue_name: String, item_id: String },
    Refreshed,
}

/// Result from queue retry operation
pub enum QueueRetryResult {
    Single {
        queue_name: String,
        item_id: String,
    },
    Bulk {
        queue_name: String,
        item_ids: Vec<String>,
        already_retried: Vec<String>,
        not_found: Vec<String>,
    },
}
