// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker event handling

mod completion;
mod dispatch;
mod lifecycle;
mod polling;

use oj_core::JobId;
use oj_runbook::QueueType;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// In-memory state for a running worker
pub(crate) struct WorkerState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub queue_name: String,
    pub job_kind: String,
    pub concurrency: u32,
    pub active_jobs: HashSet<JobId>,
    pub status: WorkerStatus,
    pub queue_type: QueueType,
    /// Maps job_id -> item_id for queue item completion tracking
    pub item_job_map: HashMap<JobId, String>,
    /// Project namespace
    pub namespace: String,
    /// Poll interval for external queues (None = no periodic polling)
    pub poll_interval: Option<String>,
    /// Number of in-flight take commands for external queues.
    /// Counted toward concurrency to prevent over-dispatch when polls overlap.
    pub pending_takes: u32,
    /// Item IDs that are in-flight (pending take or active job) for external queues.
    /// Prevents duplicate dispatches when overlapping polls return the same items.
    pub inflight_items: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerStatus {
    Running,
    Stopped,
}
