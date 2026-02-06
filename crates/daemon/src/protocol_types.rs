// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! DTO structs for the IPC protocol.

use std::collections::HashMap;
use std::path::PathBuf;

use oj_core::{StepOutcome, StepRecord};
use serde::{Deserialize, Serialize};

/// Summary of a job for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: String,
    #[serde(default)]
    pub created_at_ms: u64,
    /// Most recent activity timestamp (from step history)
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub retry_count: u32,
}

/// Detailed job information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobDetail {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: String,
    pub vars: HashMap<String, String>,
    pub workspace_path: Option<PathBuf>,
    pub session_id: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub steps: Vec<StepRecordDetail>,
    #[serde(default)]
    pub agents: Vec<AgentSummary>,
    #[serde(default)]
    pub namespace: String,
}

/// Record of a step execution for display
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepRecordDetail {
    pub name: String,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub outcome: String,
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
}

impl From<&StepRecord> for StepRecordDetail {
    fn from(r: &StepRecord) -> Self {
        StepRecordDetail {
            name: r.name.clone(),
            started_at_ms: r.started_at_ms,
            finished_at_ms: r.finished_at_ms,
            outcome: match &r.outcome {
                StepOutcome::Running => "running".to_string(),
                StepOutcome::Completed => "completed".to_string(),
                StepOutcome::Failed(_) => "failed".to_string(),
                StepOutcome::Waiting(_) => "waiting".to_string(),
            },
            detail: match &r.outcome {
                StepOutcome::Failed(e) => Some(e.clone()),
                StepOutcome::Waiting(r) => Some(r.clone()),
                _ => None,
            },
            agent_id: r.agent_id.clone(),
            agent_name: r.agent_name.clone(),
        }
    }
}

/// Detailed agent information for `oj agent show`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentDetail {
    pub agent_id: String,
    pub agent_name: Option<String>,
    pub job_id: String,
    pub job_name: String,
    pub step_name: String,
    pub namespace: Option<String>,
    pub status: String,
    pub workspace_path: Option<PathBuf>,
    pub session_id: Option<String>,
    pub files_read: usize,
    pub files_written: usize,
    pub commands_run: usize,
    pub exit_reason: Option<String>,
    pub error: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}

/// Summary of agent activity for a job step
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSummary {
    /// Job that owns this agent
    #[serde(default)]
    pub job_id: String,
    /// Step name that spawned this agent
    pub step_name: String,
    /// Agent instance ID
    pub agent_id: String,
    /// Agent name from the runbook definition
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// Project namespace
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Current status: "completed", "running", "failed", "waiting"
    pub status: String,
    /// Number of files read
    pub files_read: usize,
    /// Number of files written or edited
    pub files_written: usize,
    /// Number of commands run
    pub commands_run: usize,
    /// Exit reason (e.g. "completed", "idle (gate passed)", "failed: ...")
    pub exit_reason: Option<String>,
    /// Most recent activity timestamp (from step history)
    #[serde(default)]
    pub updated_at_ms: u64,
}

/// Summary of a session for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    #[serde(default)]
    pub namespace: String,
    pub job_id: Option<String>,
    /// Most recent activity timestamp (from associated job)
    #[serde(default)]
    pub updated_at_ms: u64,
}

/// Summary of a workspace for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceSummary {
    pub id: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub status: String,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub namespace: String,
}

/// Detailed workspace information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceDetail {
    pub id: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub owner: Option<String>,
    pub status: String,
    #[serde(default)]
    pub created_at_ms: u64,
}

/// Workspace entry for drop/prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceEntry {
    pub id: String,
    pub path: PathBuf,
    pub branch: Option<String>,
}

/// Summary of a queue item
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueItemSummary {
    pub id: String,
    pub status: String,
    pub data: HashMap<String, String>,
    pub worker_name: Option<String>,
    pub pushed_at_epoch_ms: u64,
    #[serde(default)]
    pub failure_count: u32,
}

/// Summary of a queue for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueSummary {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub queue_type: String,
    pub item_count: usize,
    pub workers: Vec<String>,
}

/// Summary of a decision for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionSummary {
    pub id: String,
    pub job_id: String,
    pub job_name: String,
    pub source: String,
    pub summary: String,
    pub created_at_ms: u64,
    #[serde(default)]
    pub namespace: String,
}

/// Detailed decision information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionDetail {
    pub id: String,
    pub job_id: String,
    pub job_name: String,
    pub agent_id: Option<String>,
    pub source: String,
    pub context: String,
    pub options: Vec<DecisionOptionDetail>,
    pub chosen: Option<usize>,
    pub message: Option<String>,
    pub created_at_ms: u64,
    pub resolved_at_ms: Option<u64>,
    #[serde(default)]
    pub namespace: String,
}

/// A single decision option for display
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionOptionDetail {
    pub number: usize,
    pub label: String,
    pub description: Option<String>,
    pub recommended: bool,
}

/// Summary of a worker for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerSummary {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub queue: String,
    pub status: String,
    pub active: usize,
    pub concurrency: u32,
    /// Most recent activity timestamp (from active jobs)
    #[serde(default)]
    pub updated_at_ms: u64,
}
