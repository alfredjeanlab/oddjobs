// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! IPC Protocol for daemon communication.
//!
//! Wire format: 4-byte length prefix (big-endian) + JSON payload

use std::collections::HashMap;
use std::path::PathBuf;

use oj_core::{AgentSignalKind, Event};
use serde::{Deserialize, Serialize};

#[path = "protocol_query.rs"]
mod query;
pub use query::Query;

#[path = "protocol_status.rs"]
mod status;
pub use status::{
    AgentEntry, AgentStatusEntry, CronEntry, CronSummary, JobEntry, JobStatusEntry,
    NamespaceStatus, OrphanAgent, OrphanSummary, ProjectSummary, QueueItemEntry, QueueStatus,
    SessionEntry, WorkerEntry,
};

#[path = "protocol_types.rs"]
mod types;
pub use types::{
    AgentDetail, AgentSummary, DecisionDetail, DecisionOptionDetail, DecisionSummary, JobDetail,
    JobSummary, QueueItemSummary, QueueSummary, SessionSummary, StepRecordDetail, WorkerSummary,
    WorkspaceDetail, WorkspaceEntry, WorkspaceSummary,
};

#[path = "protocol_wire.rs"]
mod wire;
pub use wire::{read_request, write_response, ProtocolError, DEFAULT_TIMEOUT, PROTOCOL_VERSION};
// Re-exported for CLI client usage (via `oj_daemon::protocol::*`)
#[allow(unused_imports)]
pub use wire::{decode, encode, read_message, write_message, MAX_MESSAGE_SIZE};

/// Request from CLI to daemon
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Request {
    /// Health check ping
    Ping,

    /// Version handshake
    Hello { version: String },

    /// Deliver an event to the event loop
    Event { event: Event },

    /// Query state
    Query { query: Query },

    /// Request daemon shutdown
    Shutdown {
        /// Kill all active sessions before stopping
        #[serde(default)]
        kill: bool,
    },

    /// Get daemon status
    Status,

    /// Send input to a session
    SessionSend { id: String, input: String },

    /// Send input to an agent
    AgentSend { agent_id: String, message: String },

    /// Resume monitoring for an escalated job
    JobResume {
        id: String,
        /// Message for nudge/recovery (required for agent steps)
        message: Option<String>,
        /// Variable updates to persist
        #[serde(default, alias = "input")]
        vars: HashMap<String, String>,
        /// Kill running agent and restart (still uses --resume to preserve conversation)
        #[serde(default)]
        kill: bool,
        /// Resume all escalated/failed jobs
        #[serde(default)]
        all: bool,
    },

    /// Cancel one or more running jobs
    JobCancel { ids: Vec<String> },

    /// Run a command from a project's runbook
    RunCommand {
        /// Path to the project root (.oj directory parent)
        project_root: PathBuf,
        /// Directory where the CLI was invoked (cwd), exposed as {invoke.dir}
        #[serde(default)]
        invoke_dir: PathBuf,
        /// Project namespace
        #[serde(default)]
        namespace: String,
        /// Command name to execute
        command: String,
        /// Positional arguments
        args: Vec<String>,
        /// Named arguments (key=value pairs)
        named_args: HashMap<String, String>,
    },

    /// Delete a specific workspace by ID
    WorkspaceDrop { id: String },

    /// Delete failed workspaces
    WorkspaceDropFailed,

    /// Delete all workspaces
    WorkspaceDropAll,

    /// Kill a session
    SessionKill { id: String },

    /// Capture tmux pane output for a session
    PeekSession {
        session_id: String,
        /// Whether to include ANSI color/escape codes in output
        with_color: bool,
    },

    /// Prune old terminal jobs and their log files
    JobPrune {
        /// Prune all terminal jobs regardless of age
        all: bool,
        /// Prune all failed jobs regardless of age
        #[serde(default)]
        failed: bool,
        /// Prune orphaned jobs (breadcrumb exists but no daemon state)
        #[serde(default)]
        orphans: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
        /// Filter by project namespace
        #[serde(default)]
        namespace: Option<String>,
    },

    /// Prune agent logs from terminal jobs
    AgentPrune {
        /// Prune all agents from terminal jobs regardless of age
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
    },

    /// Prune old workspaces from terminal jobs
    WorkspacePrune {
        /// Prune all terminal workspaces regardless of age
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
        /// Filter by project namespace
        #[serde(default)]
        namespace: Option<String>,
    },

    /// Prune stopped workers from daemon state
    WorkerPrune {
        /// Prune all stopped workers (accepts for consistency)
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
        /// Optional namespace filter — prune only workers in this namespace
        #[serde(default)]
        namespace: Option<String>,
    },

    /// Start a worker to process queue items
    WorkerStart {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        /// Worker name (empty string when `all` is true)
        worker_name: String,
        /// Start all workers defined in runbooks
        #[serde(default)]
        all: bool,
    },

    /// Wake a running worker to poll immediately
    WorkerWake {
        worker_name: String,
        #[serde(default)]
        namespace: String,
    },

    /// Stop a running worker
    WorkerStop {
        worker_name: String,
        #[serde(default)]
        namespace: String,
        #[serde(default)]
        project_root: Option<PathBuf>,
    },

    /// Restart a worker (stop, reload runbook, start)
    WorkerRestart {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        worker_name: String,
    },

    /// Resize a worker's concurrency at runtime
    WorkerResize {
        worker_name: String,
        #[serde(default)]
        namespace: String,
        concurrency: u32,
    },

    /// Start a cron timer
    CronStart {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        /// Cron name (empty string when `all` is true)
        cron_name: String,
        /// Start all crons defined in runbooks
        #[serde(default)]
        all: bool,
    },

    /// Stop a cron timer
    CronStop {
        cron_name: String,
        #[serde(default)]
        namespace: String,
        #[serde(default)]
        project_root: Option<PathBuf>,
    },

    /// Restart a cron (stop, reload runbook, start)
    CronRestart {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        cron_name: String,
    },

    /// Prune stopped crons from daemon state
    CronPrune {
        /// Prune all stopped crons (accepts for consistency)
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
    },

    /// Run the cron's job once immediately (no timer)
    CronOnce {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        cron_name: String,
    },

    /// Push an item to a queue (persisted: enqueue data; external: trigger poll)
    QueuePush {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
        data: serde_json::Value,
    },

    /// Drop an item from a persisted queue
    QueueDrop {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
        item_id: String,
    },

    /// Retry dead or failed queue items (bulk operation)
    QueueRetry {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
        /// Item IDs to retry (empty when using filters)
        #[serde(default)]
        item_ids: Vec<String>,
        /// Retry all dead items
        #[serde(default)]
        all_dead: bool,
        /// Retry items with specific status (dead or failed)
        #[serde(default)]
        status: Option<String>,
    },

    /// Retry multiple dead or failed queue items (bulk operation)
    QueueRetryBulk {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
        /// Specific item IDs to retry (ignored if all_dead is true)
        #[serde(default)]
        item_ids: Vec<String>,
        /// Retry all dead items in the queue
        #[serde(default)]
        all_dead: bool,
        /// Filter by status (e.g., "dead", "failed")
        #[serde(default)]
        status_filter: Option<String>,
    },

    /// Drain all pending items from a persisted queue
    QueueDrain {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
    },

    /// Force-fail an active queue item
    QueueFail {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
        item_id: String,
    },

    /// Force-complete an active queue item
    QueueDone {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
        item_id: String,
    },

    /// Prune completed/dead items from a persisted queue
    QueuePrune {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
        /// Prune all terminal items regardless of age
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
    },

    /// Resolve a pending decision
    DecisionResolve {
        id: String,
        /// 1-indexed option choice
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chosen: Option<usize>,
        /// Freeform message
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Resume all resumable jobs (waiting/failed/pending)
    JobResumeAll {
        /// Kill running agents and restart
        #[serde(default)]
        kill: bool,
    },

    /// Resume an agent (re-spawn with --resume to preserve conversation)
    AgentResume {
        /// Agent ID (full or prefix). Empty string for --all mode.
        agent_id: String,
        /// Force kill session before resuming
        #[serde(default)]
        kill: bool,
        /// Resume all dead agents
        #[serde(default)]
        all: bool,
    },

    /// Prune orphaned sessions from daemon state
    SessionPrune {
        /// Prune all orphaned sessions regardless of age
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
        /// Filter by project namespace
        #[serde(default)]
        namespace: Option<String>,
    },
}

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

    /// List of sessions
    Sessions { sessions: Vec<SessionSummary> },

    /// Single session details
    Session {
        session: Option<Box<SessionSummary>>,
    },

    /// List of workspaces
    Workspaces { workspaces: Vec<WorkspaceSummary> },

    /// Single workspace details
    Workspace {
        workspace: Option<Box<WorkspaceDetail>>,
    },

    /// Daemon status
    Status {
        uptime_secs: u64,
        jobs_active: usize,
        sessions_active: usize,
        #[serde(default)]
        orphan_count: usize,
    },

    /// Error response
    Error { message: String },

    /// Command started successfully
    CommandStarted { job_id: String, job_name: String },

    /// Standalone agent run started successfully
    AgentRunStarted {
        agent_run_id: String,
        agent_name: String,
    },

    /// Workspace(s) deleted
    WorkspacesDropped { dropped: Vec<WorkspaceEntry> },

    /// Job log contents
    JobLogs {
        /// Path to the log file (for --follow mode)
        log_path: PathBuf,
        /// Log content (most recent N lines)
        content: String,
    },

    /// Agent log contents
    AgentLogs {
        /// Path to the log file or directory (for --follow mode)
        /// Single path when step is specified, directory when all steps
        log_path: PathBuf,
        /// Log content (most recent N lines)
        content: String,
        /// Step names in order (for multi-step display)
        #[serde(default)]
        steps: Vec<String>,
    },

    /// Session pane snapshot
    SessionPeek { output: String },

    /// Job prune result
    JobsPruned {
        pruned: Vec<JobEntry>,
        skipped: usize,
    },

    /// Agent prune result
    AgentsPruned {
        pruned: Vec<AgentEntry>,
        skipped: usize,
    },

    /// Workspace prune result
    WorkspacesPruned {
        pruned: Vec<WorkspaceEntry>,
        skipped: usize,
    },

    /// Worker prune result
    WorkersPruned {
        pruned: Vec<WorkerEntry>,
        skipped: usize,
    },

    /// Cron prune result
    CronsPruned {
        pruned: Vec<CronEntry>,
        skipped: usize,
    },

    /// Queue prune result
    QueuesPruned {
        pruned: Vec<QueueItemEntry>,
        skipped: usize,
    },

    /// Response for bulk cancel operations
    JobsCancelled {
        /// IDs of successfully cancelled jobs
        cancelled: Vec<String>,
        /// IDs of jobs that were already terminal (no-op)
        already_terminal: Vec<String>,
        /// IDs that were not found
        not_found: Vec<String>,
    },

    /// Worker started successfully
    WorkerStarted { worker_name: String },

    /// Multiple workers started (--all mode)
    WorkersStarted {
        /// Workers that were started
        started: Vec<String>,
        /// Workers that were skipped with reasons
        skipped: Vec<(String, String)>,
    },

    /// Worker concurrency was updated
    WorkerResized {
        worker_name: String,
        old_concurrency: u32,
        new_concurrency: u32,
    },

    /// Cron started successfully
    CronStarted { cron_name: String },

    /// Multiple crons started (--all mode)
    CronsStarted {
        /// Crons that were started
        started: Vec<String>,
        /// Crons that were skipped with reasons
        skipped: Vec<(String, String)>,
    },

    /// List of crons
    Crons { crons: Vec<CronSummary> },

    /// Cron log contents
    CronLogs {
        /// Path to the log file (for --follow mode)
        log_path: PathBuf,
        /// Log content (most recent N lines)
        content: String,
    },

    /// Item pushed to queue (persisted) or workers woken to re-poll (external)
    QueuePushed { queue_name: String, item_id: String },

    /// Item was dropped from queue
    QueueDropped { queue_name: String, item_id: String },

    /// Item was retried (moved back to pending) - single item
    QueueRetried { queue_name: String, item_id: String },

    /// Items were retried (bulk operation)
    QueueItemsRetried {
        queue_name: String,
        /// IDs of items that were successfully retried
        item_ids: Vec<String>,
        /// IDs of items that were skipped (not dead/failed)
        already_retried: Vec<String>,
        /// Item ID prefixes that were not found
        not_found: Vec<String>,
    },

    /// Queue was drained (all pending items removed)
    QueueDrained {
        queue_name: String,
        items: Vec<QueueItemSummary>,
    },

    /// Item was force-failed
    QueueFailed { queue_name: String, item_id: String },

    /// Item was force-completed
    QueueCompleted { queue_name: String, item_id: String },

    /// Agent signal query result (for stop hook)
    AgentSignal {
        signaled: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<AgentSignalKind>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Queue items listing
    QueueItems { items: Vec<QueueItemSummary> },

    /// Worker activity log contents
    WorkerLogs {
        /// Path to the log file (for --follow mode)
        log_path: PathBuf,
        /// Log content (most recent N lines)
        content: String,
    },

    /// List of workers
    Workers { workers: Vec<WorkerSummary> },

    /// List of queues
    Queues { queues: Vec<QueueSummary> },

    /// Cross-project status overview
    StatusOverview {
        uptime_secs: u64,
        namespaces: Vec<NamespaceStatus>,
    },

    /// List of orphaned jobs detected from breadcrumbs
    Orphans { orphans: Vec<OrphanSummary> },

    /// List of projects with active work
    Projects { projects: Vec<ProjectSummary> },

    /// Queue activity log contents
    QueueLogs {
        /// Path to the log file (for --follow mode)
        log_path: PathBuf,
        /// Log content (most recent N lines)
        content: String,
    },

    /// List of decisions
    Decisions { decisions: Vec<DecisionSummary> },

    /// Single decision detail
    Decision {
        decision: Option<Box<DecisionDetail>>,
    },

    /// Decision resolved successfully
    DecisionResolved { id: String },

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

    /// Session prune result
    SessionsPruned {
        pruned: Vec<SessionEntry>,
        skipped: usize,
    },
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
