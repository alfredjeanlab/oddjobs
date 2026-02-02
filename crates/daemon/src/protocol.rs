// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! IPC Protocol for daemon communication.
//!
//! Wire format: 4-byte length prefix (big-endian) + JSON payload

use std::collections::HashMap;
use std::path::PathBuf;

use oj_core::{AgentSignalKind, Event};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

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

    /// Resume monitoring for an escalated pipeline
    PipelineResume {
        id: String,
        /// Message for nudge/recovery (required for agent steps)
        message: Option<String>,
        /// Variable updates to persist
        #[serde(default, alias = "input")]
        vars: HashMap<String, String>,
    },

    /// Cancel one or more running pipelines
    PipelineCancel { ids: Vec<String> },

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

    /// Capture tmux pane output for a session
    PeekSession {
        session_id: String,
        /// Whether to include ANSI color/escape codes in output
        with_color: bool,
    },

    /// Prune old workspaces from terminal pipelines
    WorkspacePrune {
        /// Prune all terminal workspaces regardless of age
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
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
    },

    /// Push an item to a persisted queue
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
}

/// Query types for reading daemon state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Query {
    ListPipelines,
    GetPipeline {
        id: String,
    },
    ListSessions,
    ListWorkspaces,
    GetWorkspace {
        id: String,
    },
    GetPipelineLogs {
        id: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
    },
    GetAgentLogs {
        /// Pipeline ID (not agent_id anymore)
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
    /// List items in a persisted queue
    ListQueueItems {
        queue_name: String,
    },
    /// List all workers and their status
    ListWorkers,
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

    /// List of pipelines
    Pipelines { pipelines: Vec<PipelineSummary> },

    /// Single pipeline details
    Pipeline {
        pipeline: Option<Box<PipelineDetail>>,
    },

    /// List of sessions
    Sessions { sessions: Vec<SessionSummary> },

    /// List of workspaces
    Workspaces { workspaces: Vec<WorkspaceSummary> },

    /// Single workspace details
    Workspace {
        workspace: Option<Box<WorkspaceDetail>>,
    },

    /// Daemon status
    Status {
        uptime_secs: u64,
        pipelines_active: usize,
        sessions_active: usize,
    },

    /// Error response
    Error { message: String },

    /// Command started successfully
    CommandStarted {
        pipeline_id: String,
        pipeline_name: String,
    },

    /// Workspace(s) deleted
    WorkspacesDropped { dropped: Vec<WorkspaceEntry> },

    /// Pipeline log contents
    PipelineLogs {
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

    /// Workspace prune result
    WorkspacesPruned {
        pruned: Vec<WorkspaceEntry>,
        skipped: usize,
    },

    /// Response for bulk cancel operations
    PipelinesCancelled {
        /// IDs of successfully cancelled pipelines
        cancelled: Vec<String>,
        /// IDs of pipelines that were already terminal (no-op)
        already_terminal: Vec<String>,
        /// IDs that were not found
        not_found: Vec<String>,
    },

    /// Worker started successfully
    WorkerStarted { worker_name: String },

    /// Item pushed to queue
    QueuePushed { queue_name: String, item_id: String },

    /// Item was dropped from queue
    QueueDropped { queue_name: String, item_id: String },

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

    /// List of workers
    Workers { workers: Vec<WorkerSummary> },
}

/// Summary of a pipeline for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineSummary {
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
}

/// Detailed pipeline information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineDetail {
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
    /// Agent ID that ran this step (if any)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// Summary of agent activity for a pipeline step
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSummary {
    /// Step name that spawned this agent
    pub step_name: String,
    /// Agent instance ID
    pub agent_id: String,
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
}

/// Summary of a session for listing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    pub pipeline_id: Option<String>,
    /// Most recent activity timestamp (from associated pipeline)
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
