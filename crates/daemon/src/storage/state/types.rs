// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Record types used by daemon subsystems.

pub use oj_core::{
    CronRecord, QueueItem, QueueItemStatus, QueuePollMeta, WorkerRecord, Workspace, WorkspaceType,
};

use serde::{Deserialize, Serialize};

/// A stored runbook snapshot for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRunbook {
    pub version: u32,
    pub data: serde_json::Value,
}
