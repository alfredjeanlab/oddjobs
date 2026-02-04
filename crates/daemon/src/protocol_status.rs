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
    pub pipeline: String,
    pub status: String,
    /// Human-readable time: "in 12m" for running, "3h ago" for stopped
    #[serde(default)]
    pub time: String,
}

/// Per-namespace status summary
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceStatus {
    pub namespace: String,
    /// Non-terminal pipelines (Running/Pending status)
    pub active_pipelines: Vec<PipelineStatusEntry>,
    /// Pipelines in Waiting status (escalated to human)
    pub escalated_pipelines: Vec<PipelineStatusEntry>,
    /// Orphaned pipelines detected from breadcrumb files
    pub orphaned_pipelines: Vec<PipelineStatusEntry>,
    /// Workers and their status
    pub workers: Vec<WorkerSummary>,
    /// Queue depths: (queue_name, pending_count, active_count, dead_count)
    pub queues: Vec<QueueStatus>,
    /// Currently running agents
    pub active_agents: Vec<AgentStatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineStatusEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: String,
    /// Duration since pipeline started (ms)
    pub elapsed_ms: u64,
    /// Reason pipeline is waiting (from StepOutcome::Waiting)
    pub waiting_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueStatus {
    pub name: String,
    pub pending: usize,
    pub active: usize,
    pub dead: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentStatusEntry {
    pub agent_id: String,
    pub agent_name: String,
    pub command_name: String,
    pub status: String,
}

/// Summary of an orphaned pipeline detected from a breadcrumb file
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrphanSummary {
    pub pipeline_id: String,
    pub project: String,
    pub kind: String,
    pub name: String,
    pub current_step: String,
    pub step_status: String,
    pub workspace_root: Option<PathBuf>,
    pub agents: Vec<OrphanAgent>,
    pub updated_at: String,
}

/// Agent info from an orphaned pipeline's breadcrumb
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrphanAgent {
    pub agent_id: String,
    pub session_name: Option<String>,
    pub log_path: PathBuf,
}

/// Pipeline entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineEntry {
    pub id: String,
    pub name: String,
    pub step: String,
}

/// Agent entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentEntry {
    pub agent_id: String,
    pub pipeline_id: String,
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

/// Summary of a project with active work
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectSummary {
    pub name: String,
    pub root: PathBuf,
    pub active_pipelines: usize,
    pub active_agents: usize,
    pub workers: usize,
    pub crons: usize,
}
