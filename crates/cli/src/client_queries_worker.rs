// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker and cron methods for DaemonClient.

use std::path::{Path, PathBuf};

use oj_wire::{Query, Request, Response};

use super::super::{ClientError, DaemonClient};

impl DaemonClient {
    // -- Worker commands --

    /// Start a worker
    pub async fn worker_start(
        &self,
        project_path: &Path,
        project: &str,
        worker: &str,
        all: bool,
    ) -> Result<StartResult, ClientError> {
        let request = Request::WorkerStart {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            worker: worker.to_string(),
            all,
        };
        match self.send(&request).await? {
            Response::WorkerStarted { worker } => Ok(StartResult::Single { name: worker }),
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
        project: &str,
        project_path: Option<&Path>,
        all: bool,
    ) -> Result<StopResult, ClientError> {
        let request = Request::WorkerStop {
            worker: name.to_string(),
            project: project.to_string(),
            project_path: project_path.map(|p| p.to_path_buf()),
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
            Ok(StopResult::Single { name: name.to_string() })
        }
    }

    /// Restart a worker
    pub async fn worker_restart(
        &self,
        project_path: &Path,
        project: &str,
        worker: &str,
    ) -> Result<String, ClientError> {
        let request = Request::WorkerRestart {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            worker: worker.to_string(),
        };
        match self.send(&request).await? {
            Response::WorkerStarted { worker } => Ok(worker),
            other => Self::reject(other),
        }
    }

    /// Resize a worker's concurrency
    pub async fn worker_resize(
        &self,
        worker: &str,
        project: &str,
        concurrency: u32,
    ) -> Result<(String, u32, u32), ClientError> {
        let request = Request::WorkerResize {
            worker: worker.to_string(),
            project: project.to_string(),
            concurrency,
        };
        match self.send(&request).await? {
            Response::WorkerResized { worker, old_concurrency, new_concurrency } => {
                Ok((worker, old_concurrency, new_concurrency))
            }
            other => Self::reject(other),
        }
    }

    /// List all workers
    pub async fn list_workers(&self) -> Result<Vec<oj_wire::WorkerSummary>, ClientError> {
        let request = Request::Query { query: Query::ListWorkers };
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
        project: Option<&str>,
    ) -> Result<(Vec<oj_wire::WorkerEntry>, usize), ClientError> {
        match self
            .send(&Request::WorkerPrune { all, dry_run, project: project.map(String::from) })
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
        project: &str,
        lines: usize,
        offset: u64,
        project_path: Option<&Path>,
    ) -> Result<(PathBuf, String, u64), ClientError> {
        let request = Request::Query {
            query: Query::GetWorkerLogs {
                name: name.to_string(),
                project: project.to_string(),
                lines,
                project_path: project_path.map(|p| p.to_path_buf()),
                offset,
            },
        };
        match self.send(&request).await? {
            Response::WorkerLogs { log_path, content, offset } => Ok((log_path, content, offset)),
            other => Self::reject(other),
        }
    }

    // -- Cron commands --

    /// Start a cron
    pub async fn cron_start(
        &self,
        project_path: &Path,
        project: &str,
        cron: &str,
        all: bool,
    ) -> Result<StartResult, ClientError> {
        let request = Request::CronStart {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            cron: cron.to_string(),
            all,
        };
        match self.send(&request).await? {
            Response::CronStarted { cron } => Ok(StartResult::Single { name: cron }),
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
        project: &str,
        project_path: Option<&Path>,
        all: bool,
    ) -> Result<StopResult, ClientError> {
        let request = Request::CronStop {
            cron: name.to_string(),
            project: project.to_string(),
            project_path: project_path.map(|p| p.to_path_buf()),
            all,
        };
        if all {
            match self.send(&request).await? {
                Response::CronsStopped { stopped, skipped } => {
                    Ok(StopResult::Multiple { stopped, skipped })
                }
                other => Self::reject(other),
            }
        } else {
            self.send_simple(&request).await?;
            Ok(StopResult::Single { name: name.to_string() })
        }
    }

    /// Restart a cron
    pub async fn cron_restart(
        &self,
        project_path: &Path,
        project: &str,
        name: &str,
    ) -> Result<String, ClientError> {
        let request = Request::CronRestart {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            cron: name.to_string(),
        };
        match self.send(&request).await? {
            Response::CronStarted { cron } => Ok(cron),
            other => Self::reject(other),
        }
    }

    /// Run a cron's job once immediately
    pub async fn cron_once(
        &self,
        project_path: &Path,
        project: &str,
        name: &str,
    ) -> Result<(String, String), ClientError> {
        let request = Request::CronOnce {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            cron: name.to_string(),
        };
        match self.send(&request).await? {
            Response::JobStarted { job_id, job_name } => Ok((job_id.to_string(), job_name)),
            other => Self::reject(other),
        }
    }

    /// List all crons
    pub async fn list_crons(&self) -> Result<Vec<oj_wire::CronSummary>, ClientError> {
        let request = Request::Query { query: Query::ListCrons };
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
    ) -> Result<(Vec<oj_wire::CronEntry>, usize), ClientError> {
        match self.send(&Request::CronPrune { all, dry_run }).await? {
            Response::CronsPruned { pruned, skipped } => Ok((pruned, skipped)),
            other => Self::reject(other),
        }
    }

    /// Get cron logs
    pub async fn get_cron_logs(
        &self,
        name: &str,
        project: &str,
        lines: usize,
        offset: u64,
        project_path: Option<&Path>,
    ) -> Result<(PathBuf, String, u64), ClientError> {
        let request = Request::Query {
            query: Query::GetCronLogs {
                name: name.to_string(),
                project: project.to_string(),
                lines,
                project_path: project_path.map(|p| p.to_path_buf()),
                offset,
            },
        };
        match self.send(&request).await? {
            Response::CronLogs { log_path, content, offset } => Ok((log_path, content, offset)),
            other => Self::reject(other),
        }
    }
}

/// Result from a stop operation (worker or cron)
pub enum StopResult {
    Single { name: String },
    Multiple { stopped: Vec<String>, skipped: Vec<(String, String)> },
}

/// Result from a start operation (worker or cron)
pub enum StartResult {
    Single { name: String },
    Multiple { started: Vec<String>, skipped: Vec<(String, String)> },
}
