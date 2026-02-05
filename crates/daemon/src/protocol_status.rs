// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Status overview and orphan detection types for the IPC protocol.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::WorkerSummary;

/// Summary of a cron for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronSummary {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub interval: String,
    pub job: String,
    pub status: String,
    /// Human-readable time: "in 12m" for running, "3h ago" for stopped
    #[serde(default)]
    pub time: String,
}

/// Per-namespace status summary
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceStatus {
    pub namespace: String,
    /// Non-terminal jobs (Running/Pending status)
    pub active_jobs: Vec<JobStatusEntry>,
    /// Jobs in Waiting status (escalated to human)
    pub escalated_jobs: Vec<JobStatusEntry>,
    /// Orphaned jobs detected from breadcrumb files
    pub orphaned_jobs: Vec<JobStatusEntry>,
    /// Workers and their status
    pub workers: Vec<WorkerSummary>,
    /// Queue depths: (queue_name, pending_count, active_count, dead_count)
    pub queues: Vec<QueueStatus>,
    /// Currently running agents
    pub active_agents: Vec<AgentStatusEntry>,
    /// Number of unresolved decisions in this namespace
    #[serde(default)]
    pub pending_decisions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobStatusEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: String,
    /// Duration since job started (ms)
    pub elapsed_ms: u64,
    /// Epoch ms of the most recent step activity (start or finish)
    #[serde(default)]
    pub last_activity_ms: u64,
    /// Reason job is waiting (from StepOutcome::Waiting)
    pub waiting_reason: Option<String>,
    /// Escalation source category (e.g., "idle", "error", "gate", "approval")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalate_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueStatus {
    pub name: String,
    pub pending: usize,
    pub active: usize,
    pub dead: usize,
    /// Completed item count for external queues with show_completed=true (ephemeral)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<usize>,
    /// Failed item count for external queues with show_failed=true (ephemeral)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentStatusEntry {
    pub agent_id: String,
    pub agent_name: String,
    pub command_name: String,
    pub status: String,
}

/// Summary of an orphaned job detected from a breadcrumb file
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrphanSummary {
    pub job_id: String,
    pub project: String,
    pub kind: String,
    pub name: String,
    pub current_step: String,
    pub step_status: String,
    pub workspace_root: Option<PathBuf>,
    pub agents: Vec<OrphanAgent>,
    pub updated_at: String,
}

/// Agent info from an orphaned job's breadcrumb
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrphanAgent {
    pub agent_id: String,
    pub session_name: Option<String>,
    pub log_path: PathBuf,
}

/// Job entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobEntry {
    pub id: String,
    pub name: String,
    pub step: String,
}

/// Agent entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentEntry {
    pub agent_id: String,
    pub job_id: String,
    pub step_name: String,
}
/// Worker entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerEntry {
    pub name: String,
    pub namespace: String,
}

/// Cron entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronEntry {
    pub name: String,
    pub namespace: String,
}

/// Session entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionEntry {
    pub id: String,
    pub job_id: String,
    pub namespace: String,
}

/// Queue item entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueItemEntry {
    pub queue_name: String,
    pub item_id: String,
    pub status: String,
}

/// Summary of a project with active work
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectSummary {
    pub name: String,
    pub root: PathBuf,
    pub active_jobs: usize,
    pub active_agents: usize,
    pub workers: usize,
    pub crons: usize,
}
