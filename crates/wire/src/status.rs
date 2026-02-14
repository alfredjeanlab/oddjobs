// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Status overview and orphan detection types for the IPC protocol.

use std::path::PathBuf;

use oj_core::{
    AgentId, Breadcrumb, BreadcrumbAgent, CronRecord, Job, JobId, MetricsHealth, OwnerId,
    StepStatusKind, WorkerRecord,
};
use serde::{Deserialize, Serialize};

use super::WorkerSummary;

/// Summary of a cron for listing and status display
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronSummary {
    pub name: String,
    pub project: String,
    pub interval: String,
    pub target: String,
    pub status: String,
    /// Human-readable time: "in 12m" for running, "3h ago" for stopped
    #[serde(default)]
    pub time: String,
}

/// Per-project status summary
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectStatus {
    pub project: String,
    /// Non-terminal jobs (Running/Pending status)
    pub active_jobs: Vec<JobStatusEntry>,
    /// Jobs in Waiting status (escalated to human)
    pub escalated_jobs: Vec<JobStatusEntry>,
    /// Suspended jobs (terminal but resumable)
    pub suspended_jobs: Vec<JobStatusEntry>,
    /// Orphaned jobs detected from breadcrumb files
    pub orphaned_jobs: Vec<JobStatusEntry>,
    /// Workers and their status
    pub workers: Vec<WorkerSummary>,
    /// Crons and their status
    pub crons: Vec<CronSummary>,
    /// Queue depths: (queue_name, pending_count, active_count, dead_count)
    pub queues: Vec<QueueStatus>,
    /// Currently running agents
    pub active_agents: Vec<AgentStatusEntry>,
    /// Number of unresolved decisions in this project
    pub pending_decisions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobStatusEntry {
    pub id: JobId,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: StepStatusKind,
    /// Duration since job started (ms)
    pub elapsed_ms: u64,
    /// Epoch ms of the most recent step activity (start or finish)
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentStatusEntry {
    pub agent_id: AgentId,
    pub agent_name: String,
    pub command_name: String,
    pub status: String,
}

/// Summary of an orphaned job detected from a breadcrumb file
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrphanSummary {
    pub job_id: JobId,
    pub project: String,
    pub kind: String,
    pub name: String,
    pub current_step: String,
    pub step_status: StepStatusKind,
    pub workspace_root: Option<PathBuf>,
    pub agents: Vec<OrphanAgent>,
    pub updated_at: String,
}

/// Agent info from an orphaned job's breadcrumb
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrphanAgent {
    pub agent_id: AgentId,
    pub session_name: Option<String>,
    pub log_path: PathBuf,
}

/// Job entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobEntry {
    pub id: JobId,
    pub name: String,
    pub step: String,
}

/// Agent entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentEntry {
    pub agent_id: AgentId,
    pub owner: OwnerId,
    pub step_name: String,
}
/// Worker entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerEntry {
    pub name: String,
    pub project: String,
}

/// Cron entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronEntry {
    pub name: String,
    pub project: String,
}

/// Queue item entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueItemEntry {
    pub queue: String,
    pub item_id: String,
    pub status: String,
}

/// Summary of metrics collector health for `oj status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsHealthSummary {
    pub last_collection_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
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

// --- From impls ---

impl From<&Job> for JobEntry {
    fn from(p: &Job) -> Self {
        JobEntry { id: JobId::from_string(&p.id), name: p.name.clone(), step: p.step.clone() }
    }
}

impl From<&MetricsHealth> for MetricsHealthSummary {
    fn from(mh: &MetricsHealth) -> Self {
        MetricsHealthSummary {
            last_collection_ms: mh.last_collection_ms,
            last_error: mh.last_error.clone(),
        }
    }
}

impl From<&WorkerRecord> for WorkerEntry {
    fn from(r: &WorkerRecord) -> Self {
        WorkerEntry { name: r.name.clone(), project: r.project.clone() }
    }
}

impl From<&CronRecord> for CronEntry {
    fn from(r: &CronRecord) -> Self {
        CronEntry { name: r.name.clone(), project: r.project.clone() }
    }
}

// --- Constructor methods ---

impl CronSummary {
    pub fn from_cron(c: &CronRecord, time: String) -> Self {
        CronSummary {
            name: c.name.clone(),
            project: c.project.clone(),
            interval: c.interval.clone(),
            target: c.target.to_string(),
            status: c.status.clone(),
            time,
        }
    }
}

impl JobStatusEntry {
    pub fn from_job(
        p: &Job,
        now_ms: u64,
        waiting_reason: Option<String>,
        escalate_source: Option<String>,
    ) -> Self {
        let created_at_ms = p.step_history.first().map(|r| r.started_at_ms).unwrap_or(0);
        let elapsed_ms = now_ms.saturating_sub(created_at_ms);
        let last_activity_ms =
            p.step_history.last().map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms)).unwrap_or(0);
        JobStatusEntry {
            id: JobId::from_string(&p.id),
            name: p.name.clone(),
            kind: p.kind.clone(),
            step: p.step.clone(),
            step_status: StepStatusKind::from(&p.step_status),
            elapsed_ms,
            last_activity_ms,
            waiting_reason,
            escalate_source,
        }
    }
}

impl From<&BreadcrumbAgent> for OrphanAgent {
    fn from(a: &BreadcrumbAgent) -> Self {
        OrphanAgent {
            agent_id: AgentId::from_string(&a.agent_id),
            session_name: a.session_name.clone(),
            log_path: a.log_path.clone(),
        }
    }
}

impl From<&Breadcrumb> for OrphanSummary {
    fn from(bc: &Breadcrumb) -> Self {
        OrphanSummary {
            job_id: JobId::from_string(&bc.job_id),
            project: bc.project.clone(),
            kind: bc.kind.clone(),
            name: bc.name.clone(),
            current_step: bc.current_step.clone(),
            step_status: parse_step_status_kind(&bc.step_status),
            workspace_root: bc.workspace_root.clone(),
            agents: bc.agents.iter().map(OrphanAgent::from).collect(),
            updated_at: bc.updated_at.clone(),
        }
    }
}

/// Parse a step status string from a breadcrumb into a `StepStatusKind`.
pub fn parse_step_status_kind(s: &str) -> StepStatusKind {
    match s {
        "pending" => StepStatusKind::Pending,
        "running" => StepStatusKind::Running,
        "waiting" => StepStatusKind::Waiting,
        "completed" => StepStatusKind::Completed,
        "failed" => StepStatusKind::Failed,
        _ => StepStatusKind::Orphaned,
    }
}
