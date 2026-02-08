// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job, daemon status, and run command methods for DaemonClient.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oj_daemon::{Query, Request, Response};

use super::super::{CancelResult, ClientError, DaemonClient, SuspendResult};

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

    /// Suspend one or more jobs by ID
    pub async fn job_suspend(&self, ids: &[String]) -> Result<SuspendResult, ClientError> {
        let request = Request::JobSuspend { ids: ids.to_vec() };
        match self.send(&request).await? {
            Response::JobsSuspended {
                suspended,
                already_terminal,
                not_found,
            } => Ok(SuspendResult {
                suspended,
                already_terminal,
                not_found,
            }),
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

    /// Get cross-project status overview
    pub async fn status_overview(
        &self,
    ) -> Result<
        (
            u64,
            Vec<oj_daemon::NamespaceStatus>,
            Option<oj_daemon::MetricsHealthSummary>,
        ),
        ClientError,
    > {
        let query = Request::Query {
            query: Query::StatusOverview,
        };
        match self.send(&query).await? {
            Response::StatusOverview {
                uptime_secs,
                namespaces,
                metrics_health,
            } => Ok((uptime_secs, namespaces, metrics_health)),
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
}
