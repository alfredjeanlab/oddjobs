// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker event handling

mod completion;
mod dispatch;
mod lifecycle;
mod polling;

use oj_core::PipelineId;
use oj_runbook::QueueType;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// In-memory state for a running worker
pub(crate) struct WorkerState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub queue_name: String,
    pub pipeline_kind: String,
    pub concurrency: u32,
    pub active_pipelines: HashSet<PipelineId>,
    pub status: WorkerStatus,
    pub queue_type: QueueType,
    /// Maps pipeline_id -> item_id for persisted queue item completion tracking
    pub item_pipeline_map: HashMap<PipelineId, String>,
    /// Project namespace
    pub namespace: String,
    /// Poll interval for external queues (None = no periodic polling)
    pub poll_interval: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerStatus {
    Running,
    Stopped,
}
