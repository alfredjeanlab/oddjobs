// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue and decision methods for DaemonClient.

use std::path::{Path, PathBuf};

use oj_daemon::{Query, Request, Response};

use super::super::{ClientError, DaemonClient};

impl DaemonClient {
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
        choices: Vec<usize>,
        message: Option<String>,
    ) -> Result<String, ClientError> {
        let request = Request::DecisionResolve {
            id: id.to_string(),
            chosen,
            choices,
            message,
        };
        match self.send(&request).await? {
            Response::DecisionResolved { id } => Ok(id),
            other => Self::reject(other),
        }
    }
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
