// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::PathBuf;

use oj_core::{CrewId, DecisionId, JobId};
use serde::{Deserialize, Serialize};

use super::{
    AgentDetail, AgentEntry, AgentSummary, CronEntry, CronSummary, DecisionDetail, DecisionSummary,
    JobDetail, JobEntry, JobSummary, MetricsHealthSummary, OrphanSummary, ProjectStatus,
    ProjectSummary, QueueItemEntry, QueueItemSummary, QueueSummary, WorkerEntry, WorkerSummary,
    WorkspaceDetail, WorkspaceEntry, WorkspaceSummary,
};

/// Response from daemon to CLI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Response {
    /// Generic success
    Ok,

    /// Health check response
    Pong,

    /// Version handshake response
    Hello { version: String },

    /// Daemon is shutting down
    ShuttingDown,

    /// Event was processed
    Event { accepted: bool },

    /// List of jobs
    Jobs { jobs: Vec<JobSummary> },

    /// Single job details
    Job { job: Option<Box<JobDetail>> },

    /// List of agents
    Agents { agents: Vec<AgentSummary> },

    /// Single agent details
    Agent { agent: Option<Box<AgentDetail>> },

    /// List of workspaces
    Workspaces { workspaces: Vec<WorkspaceSummary> },

    /// Single workspace details
    Workspace { workspace: Option<Box<WorkspaceDetail>> },

    /// Daemon status
    Status {
        uptime_secs: u64,
        jobs_active: usize,
        #[serde(default)]
        orphan_count: usize,
    },

    /// Error response
    Error { message: String },

    /// Command started successfully
    JobStarted { job_id: JobId, job_name: String },

    /// Crew started successfully
    CrewStarted { crew_id: CrewId, agent_name: String },

    /// Workspace(s) deleted
    WorkspacesDropped { dropped: Vec<WorkspaceEntry> },

    /// Job log contents
    JobLogs {
        /// Path to the log file (for --follow mode)
        log_path: PathBuf,
        /// Log content (most recent N lines, or from offset)
        content: String,
        /// Byte offset after this content (for incremental polling)
        #[serde(default)]
        offset: u64,
    },

    /// Agent log contents
    AgentLogs {
        /// Path to the log file or directory (for --follow mode)
        /// Single path when step is specified, directory when all steps
        log_path: PathBuf,
        /// Log content (most recent N lines, or from offset)
        content: String,
        /// Step names in order (for multi-step display)
        #[serde(default)]
        steps: Vec<String>,
        /// Byte offset after this content (for incremental polling)
        #[serde(default)]
        offset: u64,
    },

    /// Job prune result
    JobsPruned { pruned: Vec<JobEntry>, skipped: usize },

    /// Agent prune result
    AgentsPruned { pruned: Vec<AgentEntry>, skipped: usize },

    /// Workspace prune result
    WorkspacesPruned { pruned: Vec<WorkspaceEntry>, skipped: usize },

    /// Worker prune result
    WorkersPruned { pruned: Vec<WorkerEntry>, skipped: usize },

    /// Cron prune result
    CronsPruned { pruned: Vec<CronEntry>, skipped: usize },

    /// Queue prune result
    QueuesPruned { pruned: Vec<QueueItemEntry>, skipped: usize },

    /// Response for bulk cancel operations
    JobsCancelled {
        /// IDs of successfully cancelled jobs
        cancelled: Vec<String>,
        /// IDs of jobs that were already terminal (no-op)
        already_terminal: Vec<String>,
        /// IDs that were not found
        not_found: Vec<String>,
    },

    /// Response for bulk suspend operations
    JobsSuspended {
        /// IDs of successfully suspended jobs
        suspended: Vec<String>,
        /// IDs of jobs that were already terminal (no-op)
        already_terminal: Vec<String>,
        /// IDs that were not found
        not_found: Vec<String>,
    },

    /// Worker started successfully
    WorkerStarted { worker: String },

    /// Multiple workers started (--all mode)
    WorkersStarted {
        /// Workers that were started
        started: Vec<String>,
        /// Workers that were skipped with reasons
        skipped: Vec<(String, String)>,
    },

    /// Multiple workers stopped (--all mode)
    WorkersStopped {
        /// Workers that were stopped
        stopped: Vec<String>,
        /// Workers that were skipped with reasons
        skipped: Vec<(String, String)>,
    },

    /// Worker concurrency was updated
    WorkerResized { worker: String, old_concurrency: u32, new_concurrency: u32 },

    /// Cron started successfully
    CronStarted { cron: String },

    /// Multiple crons started (--all mode)
    CronsStarted {
        /// Crons that were started
        started: Vec<String>,
        /// Crons that were skipped with reasons
        skipped: Vec<(String, String)>,
    },

    /// Multiple crons stopped (--all mode)
    CronsStopped {
        /// Crons that were stopped
        stopped: Vec<String>,
        /// Crons that were skipped with reasons
        skipped: Vec<(String, String)>,
    },

    /// List of crons
    Crons { crons: Vec<CronSummary> },

    /// Cron log contents
    CronLogs {
        /// Path to the log file (for --follow mode)
        log_path: PathBuf,
        /// Log content (most recent N lines, or from offset)
        content: String,
        /// Byte offset after this content (for incremental polling)
        #[serde(default)]
        offset: u64,
    },

    /// Item pushed to queue (persisted) or workers woken to re-poll (external)
    QueuePushed { queue: String, item_id: String },

    /// Item was dropped from queue
    QueueDropped { queue: String, item_id: String },

    /// Items were retried
    QueueRetried {
        queue: String,
        /// IDs of items that were successfully retried
        item_ids: Vec<String>,
        /// IDs of items that were skipped (not dead/failed)
        already_retried: Vec<String>,
        /// Item ID prefixes that were not found
        not_found: Vec<String>,
    },

    /// Queue was drained (all pending items removed)
    QueueDrained { queue: String, items: Vec<QueueItemSummary> },

    /// Item was force-failed
    QueueFailed { queue: String, item_id: String },

    /// Item was force-completed
    QueueCompleted { queue: String, item_id: String },

    /// Queue items listing
    QueueItems { items: Vec<QueueItemSummary> },

    /// Worker activity log contents
    WorkerLogs {
        /// Path to the log file (for --follow mode)
        log_path: PathBuf,
        /// Log content (most recent N lines, or from offset)
        content: String,
        /// Byte offset after this content (for incremental polling)
        #[serde(default)]
        offset: u64,
    },

    /// List of workers
    Workers { workers: Vec<WorkerSummary> },

    /// List of queues
    Queues { queues: Vec<QueueSummary> },

    /// Cross-project status overview
    StatusOverview {
        uptime_secs: u64,
        projects: Vec<ProjectStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metrics_health: Option<MetricsHealthSummary>,
    },

    /// List of orphaned jobs detected from breadcrumbs
    Orphans { orphans: Vec<OrphanSummary> },

    /// List of projects with active work
    Projects { projects: Vec<ProjectSummary> },

    /// Queue activity log contents
    QueueLogs {
        /// Path to the log file (for --follow mode)
        log_path: PathBuf,
        /// Log content (most recent N lines, or from offset)
        content: String,
        /// Byte offset after this content (for incremental polling)
        #[serde(default)]
        offset: u64,
    },

    /// List of decisions
    Decisions { decisions: Vec<DecisionSummary> },

    /// Single decision detail
    Decision { decision: Option<Box<DecisionDetail>> },

    /// Decision resolved successfully
    DecisionResolved { id: DecisionId },

    /// Result of agent resume
    AgentResumed {
        /// Agents that were resumed (agent_id list)
        resumed: Vec<String>,
        /// Agents that were skipped with reasons
        skipped: Vec<(String, String)>,
    },

    /// Result of bulk job resume
    JobsResumed {
        /// Job IDs that were resumed
        resumed: Vec<String>,
        /// Jobs that were skipped with reasons (id, reason)
        skipped: Vec<(String, String)>,
    },

    /// Connection is ready for raw byte streaming to agent's terminal (remote proxy)
    AgentAttachReady { id: String },
    /// Agent is local — CLI should attach directly via socket path
    AgentAttachLocal { id: String, socket_path: String },
}

#[cfg(test)]
#[path = "response_tests.rs"]
mod tests;
