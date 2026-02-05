// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! IPC Protocol for daemon communication.
//!
//! Wire format: 4-byte length prefix (big-endian) + JSON payload

use std::collections::HashMap;
use std::path::PathBuf;

use oj_core::{AgentSignalKind, Event, StepOutcome, StepRecord};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

#[path = "protocol_status.rs"]
mod status;
pub use status::{
    AgentEntry, AgentStatusEntry, CronEntry, CronSummary, JobEntry, JobStatusEntry,
    NamespaceStatus, OrphanAgent, OrphanSummary, ProjectSummary, QueueItemEntry, QueueStatus,
    SessionEntry, WorkerEntry,
};

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
        worker_name: String,
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

    /// Start a cron timer
    CronStart {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        cron_name: String,
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

    /// Retry a dead or failed queue item
    QueueRetry {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
        queue_name: String,
        item_id: String,
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

/// Query types for reading daemon state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Query {
    ListJobs,
    GetJob {
        id: String,
    },
    ListSessions,
    /// Get a single session by ID (exact or prefix match)
    GetSession {
        id: String,
    },
    ListWorkspaces,
    GetWorkspace {
        id: String,
    },
    GetJobLogs {
        id: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
    },
    GetAgentLogs {
        /// Job ID (not agent_id anymore)
        id: String,
        /// Optional step filter (None = all steps)
        #[serde(default)]
        step: Option<String>,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
    },
    /// Query if an agent has signaled completion (for stop hook)
    GetAgentSignal {
        agent_id: String,
    },
    /// List all known queues in a project
    ListQueues {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
    },
    /// List items in a persisted queue
    ListQueueItems {
        queue_name: String,
        #[serde(default)]
        namespace: String,
        #[serde(default)]
        project_root: Option<PathBuf>,
    },
    /// Get detailed info for a single agent by ID (or prefix)
    GetAgent {
        agent_id: String,
    },
    /// List agents across all jobs
    ListAgents {
        /// Filter by job ID prefix
        #[serde(default)]
        job_id: Option<String>,
        /// Filter by status (e.g. "running", "completed", "failed", "waiting")
        #[serde(default)]
        status: Option<String>,
    },
    /// Get worker activity logs
    GetWorkerLogs {
        name: String,
        #[serde(default)]
        namespace: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        #[serde(default)]
        project_root: Option<PathBuf>,
    },
    /// List all workers and their status
    ListWorkers,
    /// List all crons and their status
    ListCrons,
    /// Get cron activity logs
    GetCronLogs {
        /// Cron name
        name: String,
        #[serde(default)]
        namespace: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        #[serde(default)]
        project_root: Option<PathBuf>,
    },
    /// Get a cross-project status overview
    StatusOverview,
    /// List all projects with active work
    ListProjects,
    /// List orphaned jobs detected from breadcrumbs at startup
    ListOrphans,
    /// Dismiss an orphaned job by ID
    DismissOrphan {
        id: String,
    },
    /// Get queue activity logs
    GetQueueLogs {
        queue_name: String,
        #[serde(default)]
        namespace: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
    },
    /// List pending decisions (optionally filtered by namespace)
    ListDecisions {
        #[serde(default)]
        namespace: String,
    },
    /// Get a single decision by ID (prefix match supported)
    GetDecision {
        id: String,
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

    /// Cron started successfully
    CronStarted { cron_name: String },

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

    /// Item was retried (moved back to pending)
    QueueRetried { queue_name: String, item_id: String },

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

    /// Session prune result
    SessionsPruned {
        pruned: Vec<SessionEntry>,
        skipped: usize,
    },
}

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

/// Protocol errors
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Message too large: {size} bytes (max {max})")]
    MessageTooLarge { size: usize, max: usize },

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Timeout")]
    Timeout,
}

/// Maximum message size (200 MB)
pub const MAX_MESSAGE_SIZE: usize = 200 * 1024 * 1024;

/// Default IPC timeout
pub const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Protocol version (from Cargo.toml)
pub const PROTOCOL_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH"));

/// Encode a message to JSON bytes (without length prefix)
///
/// Use with `write_message()` which handles the length-prefix wire format.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, ProtocolError> {
    let json = serde_json::to_vec(msg)?;

    if json.len() > MAX_MESSAGE_SIZE {
        return Err(ProtocolError::MessageTooLarge {
            size: json.len(),
            max: MAX_MESSAGE_SIZE,
        });
    }

    Ok(json)
}

/// Decode a message from wire format
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolError> {
    Ok(serde_json::from_slice(bytes)?)
}

/// Read a length-prefixed message from an async reader
pub async fn read_message<R: tokio::io::AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, ProtocolError> {
    // Read length prefix
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ProtocolError::ConnectionClosed);
        }
        Err(e) => return Err(ProtocolError::Io(e)),
    }
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(ProtocolError::MessageTooLarge {
            size: len,
            max: MAX_MESSAGE_SIZE,
        });
    }

    // Read payload
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Write a length-prefixed message to an async writer
pub async fn write_message<W: tokio::io::AsyncWriteExt + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<(), ProtocolError> {
    let len = data.len();
    if len > MAX_MESSAGE_SIZE {
        return Err(ProtocolError::MessageTooLarge {
            size: len,
            max: MAX_MESSAGE_SIZE,
        });
    }

    writer.write_all(&(len as u32).to_be_bytes()).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a request with timeout
pub async fn read_request<R: tokio::io::AsyncReadExt + Unpin>(
    reader: &mut R,
    timeout: std::time::Duration,
) -> Result<Request, ProtocolError> {
    let bytes = tokio::time::timeout(timeout, read_message(reader))
        .await
        .map_err(|_| ProtocolError::Timeout)??;
    decode(&bytes)
}

/// Write a response with timeout
pub async fn write_response<W: tokio::io::AsyncWriteExt + Unpin>(
    writer: &mut W,
    response: &Response,
    timeout: std::time::Duration,
) -> Result<(), ProtocolError> {
    let data = encode(response)?;
    tokio::time::timeout(timeout, write_message(writer, &data))
        .await
        .map_err(|_| ProtocolError::Timeout)?
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
