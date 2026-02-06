// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Pure type definitions for materialized state records.

use oj_core::{OwnerId, WorkspaceStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Session record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub job_id: String,
}

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
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["folder", "worktree"],
            )),
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
    /// Owner of the workspace (job or agent_run)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<OwnerId>,
    /// Current lifecycle status
    pub status: WorkspaceStatus,
    /// Workspace type (folder or worktree)
    #[serde(default)]
    pub workspace_type: WorkspaceType,
    /// Epoch milliseconds when workspace was created (0 for pre-existing workspaces)
    #[serde(default)]
    pub created_at_ms: u64,
}

/// A stored runbook snapshot for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRunbook {
    pub version: u32,
    pub data: serde_json::Value,
}

/// Record of a running worker for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRecord {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub project_root: PathBuf,
    pub runbook_hash: String,
    /// "running" or "stopped"
    pub status: String,
    #[serde(default)]
    pub active_job_ids: Vec<String>,
    #[serde(default)]
    pub queue_name: String,
    #[serde(default)]
    pub concurrency: u32,
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
    pub queue_name: String,
    pub data: HashMap<String, String>,
    pub status: QueueItemStatus,
    pub worker_name: Option<String>,
    pub pushed_at_epoch_ms: u64,
    /// Number of times this item has failed (for retry tracking)
    #[serde(default)]
    pub failure_count: u32,
}

/// Record of a running cron for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRecord {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub project_root: PathBuf,
    pub runbook_hash: String,
    /// "running" or "stopped"
    pub status: String,
    pub interval: String,
    /// What this cron runs: "job:name" or "agent:name"
    pub run_target: String,
    /// Epoch ms when the cron was started (timer began)
    #[serde(default)]
    pub started_at_ms: u64,
    /// Epoch ms when the cron last fired (spawned a job)
    #[serde(default)]
    pub last_fired_at_ms: Option<u64>,
}
