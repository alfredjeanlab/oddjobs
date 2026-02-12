// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::path::PathBuf;

use oj_core::Event;
use serde::{Deserialize, Serialize};

use super::Query;

/// Request from CLI to daemon
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Request {
    /// Health check ping
    Ping,

    /// Version handshake
    Hello {
        version: String,
        /// Auth token for TCP connections (ignored for Unix socket)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },

    /// Deliver an event to the event loop
    Event { event: Event },

    /// Query state
    Query { query: Query },

    /// Request daemon shutdown
    Shutdown {
        /// Kill all active agent sessions before stopping
        #[serde(default)]
        kill: bool,
    },

    /// Get daemon status
    Status,

    /// Send input to an agent
    AgentSend { id: String, message: String },

    /// Resume monitoring for an escalated job
    JobResume {
        id: String,
        /// Message for nudge/recovery (required for agent steps)
        message: Option<String>,
        /// Variable updates to persist
        #[serde(default)]
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

    /// Suspend one or more running jobs
    JobSuspend { ids: Vec<String> },

    /// Run a command from a project's runbook
    RunCommand {
        /// Project project
        project: String,
        /// Path to the project root (.oj directory parent)
        project_path: PathBuf,
        /// Directory where the CLI was invoked (cwd), exposed as {invoke.dir}
        invoke_dir: PathBuf,
        /// Command name to execute
        command: String,
        /// Positional arguments
        args: Vec<String>,
        /// Named arguments (key=value pairs)
        kwargs: HashMap<String, String>,
    },

    /// Delete a specific workspace by ID
    WorkspaceDrop { id: String },

    /// Delete failed workspaces
    WorkspaceDropFailed,

    /// Delete all workspaces
    WorkspaceDropAll,

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
        /// Filter by project project
        #[serde(default)]
        project: Option<String>,
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
        /// Filter by project project
        #[serde(default)]
        project: Option<String>,
    },

    /// Prune stopped workers from daemon state
    WorkerPrune {
        /// Prune all stopped workers (accepts for consistency)
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
        /// Optional project filter — prune only workers in this project
        #[serde(default)]
        project: Option<String>,
    },

    /// Start a worker to process queue items
    WorkerStart {
        project_path: PathBuf,
        project: String,
        /// Worker name (empty string when `all` is true)
        worker: String,
        /// Start all workers defined in runbooks
        #[serde(default)]
        all: bool,
    },

    /// Wake a running worker to poll immediately
    WorkerWake { worker: String, project: String },

    /// Stop a running worker
    WorkerStop {
        /// Worker name (empty string when `all` is true)
        worker: String,
        project: String,
        #[serde(default)]
        project_path: Option<PathBuf>,
        /// Stop all running workers in the project
        #[serde(default)]
        all: bool,
    },

    /// Restart a worker (stop, reload runbook, start)
    WorkerRestart { worker: String, project: String, project_path: PathBuf },

    /// Resize a worker's concurrency at runtime
    WorkerResize { worker: String, project: String, concurrency: u32 },

    /// Start a cron timer
    CronStart {
        project: String,
        project_path: PathBuf,
        /// Cron name (empty string when `all` is true)
        cron: String,
        /// Start all crons defined in runbooks
        #[serde(default)]
        all: bool,
    },

    /// Stop a cron timer
    CronStop {
        /// Cron name (empty string when `all` is true)
        cron: String,
        project: String,
        project_path: Option<PathBuf>,
        /// Stop all running crons in the project
        #[serde(default)]
        all: bool,
    },

    /// Restart a cron (stop, reload runbook, start)
    CronRestart { cron: String, project: String, project_path: PathBuf },

    /// Prune stopped crons from daemon state
    CronPrune {
        /// Prune all stopped crons (accepts for consistency)
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
    },

    /// Run the cron's job once immediately (no timer)
    CronOnce { cron: String, project: String, project_path: PathBuf },

    /// Push an item to a queue (persisted: enqueue data; external: trigger poll)
    QueuePush { queue: String, project: String, project_path: PathBuf, data: serde_json::Value },

    /// Drop an item from a persisted queue
    QueueDrop { queue: String, project: String, project_path: PathBuf, item_id: String },

    /// Retry dead or failed queue items (bulk operation)
    QueueRetry {
        queue: String,
        project: String,
        project_path: PathBuf,
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

    /// Drain all pending items from a persisted queue
    QueueDrain { queue: String, project: String, project_path: PathBuf },

    /// Force-fail an active queue item
    QueueFail { queue: String, project_path: PathBuf, project: String, item_id: String },

    /// Force-complete an active queue item
    QueueDone { queue: String, project: String, project_path: PathBuf, item_id: String },

    /// Prune completed/dead items from a persisted queue
    QueuePrune {
        project: String,
        project_path: PathBuf,
        queue: String,
        /// Prune all terminal items regardless of age
        all: bool,
        /// Preview only -- don't actually delete
        dry_run: bool,
    },

    /// Resolve a pending decision
    DecisionResolve {
        id: String,
        /// 1-indexed option choices (single element for single-question, multiple for multi-question)
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<usize>,
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
        id: String,
        /// Force kill session before resuming
        #[serde(default)]
        kill: bool,
        /// Resume all dead agents
        #[serde(default)]
        all: bool,
    },

    /// Kill an agent's session (triggers on_dead lifecycle)
    AgentKill { id: String },

    /// Attach to a remote agent's terminal via daemon proxy.
    ///
    /// After the handshake, the connection switches to raw byte mode —
    /// the daemon bridges CLI ↔ coop WebSocket.
    AgentAttach {
        id: String,
        /// Auth token for TCP connections (unused for Unix sockets)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
    },
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
