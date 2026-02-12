// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Pure type definitions for materialized state records.

use crate::{OwnerId, RunTarget, WorkspaceStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Workspace type for lifecycle management
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceType {
    /// Plain directory — engine creates/deletes the directory
    #[default]
    Folder,
    /// Git worktree — engine manages worktree add/remove and branch lifecycle
    Worktree,
}

impl serde::Serialize for WorkspaceType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            WorkspaceType::Folder => serializer.serialize_str("folder"),
            WorkspaceType::Worktree => serializer.serialize_str("worktree"),
        }
    }
}

impl<'de> serde::Deserialize<'de> for WorkspaceType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "folder" => Ok(WorkspaceType::Folder),
            "worktree" => Ok(WorkspaceType::Worktree),
            other => Err(serde::de::Error::unknown_variant(other, &["folder", "worktree"])),
        }
    }
}

/// Workspace record with lifecycle management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub path: PathBuf,
    /// Branch for the worktree (None for folder workspaces)
    pub branch: Option<String>,
    /// Owner of the workspace (job or crew)
    pub owner: OwnerId,
    /// Current lifecycle status
    pub status: WorkspaceStatus,
    /// Workspace type (folder or worktree)
    pub workspace_type: WorkspaceType,
    /// Epoch milliseconds when workspace was created (0 for pre-existing workspaces)
    pub created_at_ms: u64,
}

/// Record of a running worker for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRecord {
    pub name: String,
    pub project: String,
    pub project_path: PathBuf,
    pub runbook_hash: String,
    /// "running" or "stopped"
    pub status: String,
    pub active: Vec<String>,
    pub queue: String,
    pub concurrency: u32,
    /// Mapping from owner → item_id for queue item tracking.
    /// Persisted via WorkerDispatched events for restart recovery.
    pub owners: HashMap<String, String>,
}

/// Status of a queue item through its lifecycle
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueItemStatus {
    Pending,
    Active,
    Completed,
    Failed,
    Dead,
}

impl std::fmt::Display for QueueItemStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueItemStatus::Pending => write!(f, "pending"),
            QueueItemStatus::Active => write!(f, "active"),
            QueueItemStatus::Completed => write!(f, "completed"),
            QueueItemStatus::Failed => write!(f, "failed"),
            QueueItemStatus::Dead => write!(f, "dead"),
        }
    }
}

/// A single item in a persisted queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub queue: String,
    pub data: HashMap<String, String>,
    pub status: QueueItemStatus,
    pub worker: Option<String>,
    pub pushed_at_ms: u64,
    /// Number of times this item has failed (for retry tracking)
    pub failures: u32,
}

/// Runtime-only metadata from the most recent queue poll.
#[derive(Debug, Default, Clone)]
pub struct QueuePollMeta {
    pub last_item_count: usize,
    pub last_polled_at_ms: u64,
}

/// Record of a running cron for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRecord {
    pub name: String,
    pub project: String,
    pub project_path: PathBuf,
    pub runbook_hash: String,
    /// "running" or "stopped"
    pub status: String,
    pub interval: String,
    pub target: RunTarget,
    /// Epoch ms when the cron was started (timer began)
    pub started_at_ms: u64,
    /// Epoch ms when the cron last fired (spawned a job)
    pub last_fired_at_ms: Option<u64>,
}
