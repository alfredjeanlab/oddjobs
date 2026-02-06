// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker and cron methods for DaemonClient.

use std::path::{Path, PathBuf};

use oj_daemon::{Query, Request, Response};

use super::super::{ClientError, DaemonClient};

impl DaemonClient {
    // -- Worker commands --

    /// Start a worker
    pub async fn worker_start(
        &self,
        project_root: &Path,
        namespace: &str,
        worker_name: &str,
        all: bool,
    ) -> Result<StartResult, ClientError> {
        let request = Request::WorkerStart {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            worker_name: worker_name.to_string(),
            all,
        };
        match self.send(&request).await? {
            Response::WorkerStarted { worker_name } => {
                Ok(StartResult::Single { name: worker_name })
            }
            Response::WorkersStarted { started, skipped } => {
                Ok(StartResult::Multiple { started, skipped })
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
        all: bool,
    ) -> Result<StopResult, ClientError> {
        let request = Request::WorkerStop {
            worker_name: name.to_string(),
            namespace: namespace.to_string(),
            project_root: project_root.map(|p| p.to_path_buf()),
            all,
        };
        if all {
            match self.send(&request).await? {
                Response::WorkersStopped { stopped, skipped } => {
                    Ok(StopResult::Multiple { stopped, skipped })
                }
                other => Self::reject(other),
            }
        } else {
            self.send_simple(&request).await?;
            Ok(StopResult::Single {
                name: name.to_string(),
            })
        }
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

    // -- Cron commands --

    /// Start a cron
    pub async fn cron_start(
        &self,
        project_root: &Path,
        namespace: &str,
        cron_name: &str,
        all: bool,
    ) -> Result<StartResult, ClientError> {
        let request = Request::CronStart {
            project_root: project_root.to_path_buf(),
            namespace: namespace.to_string(),
            cron_name: cron_name.to_string(),
            all,
        };
        match self.send(&request).await? {
            Response::CronStarted { cron_name } => Ok(StartResult::Single { name: cron_name }),
            Response::CronsStarted { started, skipped } => {
                Ok(StartResult::Multiple { started, skipped })
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
}

/// Result from a stop operation (worker or cron)
pub enum StopResult {
    Single {
        name: String,
    },
    Multiple {
        stopped: Vec<String>,
        skipped: Vec<(String, String)>,
    },
}

/// Result from a start operation (worker or cron)
pub enum StartResult {
    Single {
        name: String,
    },
    Multiple {
        started: Vec<String>,
        skipped: Vec<(String, String)>,
    },
}
