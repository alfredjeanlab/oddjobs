// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue and decision methods for DaemonClient.

use std::path::{Path, PathBuf};

use oj_wire::{Query, Request, Response};

use super::super::{ClientError, DaemonClient};

impl DaemonClient {
    // -- Queue commands --

    /// Push an item to a queue
    pub async fn queue_push(
        &self,
        project_path: &Path,
        project: &str,
        queue: &str,
        data: serde_json::Value,
    ) -> Result<QueuePushResult, ClientError> {
        let request = Request::QueuePush {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            queue: queue.to_string(),
            data,
        };
        match self.send(&request).await? {
            Response::QueuePushed { queue, item_id } => {
                Ok(QueuePushResult::Pushed { queue, item_id })
            }
            Response::Ok => Ok(QueuePushResult::Refreshed),
            other => Self::reject(other),
        }
    }

    /// Drop an item from a queue
    pub async fn queue_drop(
        &self,
        project_path: &Path,
        project: &str,
        queue: &str,
        item_id: &str,
    ) -> Result<(String, String), ClientError> {
        let request = Request::QueueDrop {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            queue: queue.to_string(),
            item_id: item_id.to_string(),
        };
        match self.send(&request).await? {
            Response::QueueDropped { queue, item_id } => Ok((queue, item_id)),
            other => Self::reject(other),
        }
    }

    /// Retry dead or failed queue items
    pub async fn queue_retry(
        &self,
        project_path: &Path,
        project: &str,
        queue: &str,
        item_ids: Vec<String>,
        all_dead: bool,
        status: Option<String>,
    ) -> Result<QueueRetryResult, ClientError> {
        let request = Request::QueueRetry {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            queue: queue.to_string(),
            item_ids,
            all_dead,
            status,
        };
        match self.send(&request).await? {
            Response::QueueRetried { queue, item_ids, already_retried, not_found } => {
                Ok(QueueRetryResult { queue, item_ids, already_retried, not_found })
            }
            other => Self::reject(other),
        }
    }

    /// Force-fail an active queue item
    pub async fn queue_fail(
        &self,
        project_path: &Path,
        project: &str,
        queue: &str,
        item_id: &str,
    ) -> Result<(String, String), ClientError> {
        let request = Request::QueueFail {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            queue: queue.to_string(),
            item_id: item_id.to_string(),
        };
        match self.send(&request).await? {
            Response::QueueFailed { queue, item_id } => Ok((queue, item_id)),
            other => Self::reject(other),
        }
    }

    /// Force-complete an active queue item
    pub async fn queue_done(
        &self,
        project_path: &Path,
        project: &str,
        queue: &str,
        item_id: &str,
    ) -> Result<(String, String), ClientError> {
        let request = Request::QueueDone {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            queue: queue.to_string(),
            item_id: item_id.to_string(),
        };
        match self.send(&request).await? {
            Response::QueueCompleted { queue, item_id } => Ok((queue, item_id)),
            other => Self::reject(other),
        }
    }

    /// Drain all pending items from a queue
    pub async fn queue_drain(
        &self,
        project_path: &Path,
        project: &str,
        queue: &str,
    ) -> Result<(String, Vec<oj_wire::QueueItemSummary>), ClientError> {
        let request = Request::QueueDrain {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            queue: queue.to_string(),
        };
        match self.send(&request).await? {
            Response::QueueDrained { queue, items } => Ok((queue, items)),
            other => Self::reject(other),
        }
    }

    /// List all queues in a project
    pub async fn list_queues(
        &self,
        project_path: &Path,
        project: &str,
    ) -> Result<Vec<oj_wire::QueueSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListQueues {
                project_path: project_path.to_path_buf(),
                project: project.to_string(),
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
        queue: &str,
        project: &str,
        project_path: Option<&Path>,
    ) -> Result<Vec<oj_wire::QueueItemSummary>, ClientError> {
        let request = Request::Query {
            query: Query::ListQueueItems {
                queue: queue.to_string(),
                project: project.to_string(),
                project_path: project_path.map(|p| p.to_path_buf()),
            },
        };
        match self.send(&request).await? {
            Response::QueueItems { items } => Ok(items),
            other => Self::reject(other),
        }
    }

    /// Prune completed/dead items from a queue
    pub async fn queue_prune(
        &self,
        project_path: &Path,
        project: &str,
        queue: &str,
        all: bool,
        dry_run: bool,
    ) -> Result<(Vec<oj_wire::QueueItemEntry>, usize), ClientError> {
        let req = Request::QueuePrune {
            project_path: project_path.to_path_buf(),
            project: project.to_string(),
            queue: queue.to_string(),
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
        queue: &str,
        project: &str,
        lines: usize,
        offset: u64,
    ) -> Result<(PathBuf, String, u64), ClientError> {
        let request = Request::Query {
            query: Query::GetQueueLogs {
                queue: queue.to_string(),
                project: project.to_string(),
                lines,
                offset,
            },
        };
        match self.send(&request).await? {
            Response::QueueLogs { log_path, content, offset } => Ok((log_path, content, offset)),
            other => Self::reject(other),
        }
    }

    // -- Decision commands --

    /// List pending decisions
    pub async fn list_decisions(
        &self,
        project: &str,
    ) -> Result<Vec<oj_wire::DecisionSummary>, ClientError> {
        let request =
            Request::Query { query: Query::ListDecisions { project: project.to_string() } };
        match self.send(&request).await? {
            Response::Decisions { decisions } => Ok(decisions),
            other => Self::reject(other),
        }
    }

    /// Get a single decision by ID
    pub async fn get_decision(
        &self,
        id: &str,
    ) -> Result<Option<oj_wire::DecisionDetail>, ClientError> {
        let request =
            Request::Query { query: Query::GetDecision { id: oj_core::DecisionId::new(id) } };
        match self.send(&request).await? {
            Response::Decision { decision } => Ok(decision.map(|b| *b)),
            other => Self::reject(other),
        }
    }

    /// Resolve a pending decision
    pub async fn decision_resolve(
        &self,
        id: &str,
        choices: Vec<usize>,
        message: Option<String>,
    ) -> Result<oj_core::DecisionId, ClientError> {
        let request =
            Request::DecisionResolve { id: oj_core::DecisionId::new(id), choices, message };
        match self.send(&request).await? {
            Response::DecisionResolved { id } => Ok(id),
            other => Self::reject(other),
        }
    }
}

/// Result from queue push operation
pub enum QueuePushResult {
    Pushed { queue: String, item_id: String },
    Refreshed,
}

/// Result from queue retry operation
pub struct QueueRetryResult {
    pub queue: String,
    pub item_ids: Vec<String>,
    pub already_retried: Vec<String>,
    pub not_found: Vec<String>,
}
