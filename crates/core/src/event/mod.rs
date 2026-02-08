// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event types for the Odd Jobs system

mod agent;
mod agent_run;
mod core_types;
mod cron_scheduler;
mod decision;
mod dispatch;
mod job;
mod step;
mod worker_queue;
mod workspace;

pub use agent::{AgentSignalKind, PromptType, QuestionData, QuestionEntry, QuestionOption};

use crate::agent::{AgentError, AgentId};
use crate::agent_run::{AgentRunId, AgentRunStatus};
use crate::decision::{DecisionOption, DecisionSource};
use crate::job::JobId;
use crate::owner::OwnerId;
use crate::session::SessionId;
use crate::timer::TimerId;
use crate::workspace::WorkspaceId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Returns ` ns={namespace}` when non-empty, empty string otherwise.
fn ns_fragment(namespace: &str) -> String {
    if namespace.is_empty() {
        String::new()
    } else {
        format!(" ns={namespace}")
    }
}

/// Events that trigger state transitions in the system.
///
/// Serializes with `{"type": "event:name", ...fields}` format.
/// Unknown type tags deserialize to `Custom`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    // -- agent --
    #[serde(rename = "agent:working")]
    AgentWorking {
        agent_id: AgentId,
        /// Owner of this agent (job or agent_run).
        owner: OwnerId,
    },

    #[serde(rename = "agent:waiting")]
    AgentWaiting {
        agent_id: AgentId,
        /// Owner of this agent (job or agent_run).
        owner: OwnerId,
    },

    #[serde(rename = "agent:failed")]
    AgentFailed {
        agent_id: AgentId,
        error: AgentError,
        /// Owner of this agent (job or agent_run).
        owner: OwnerId,
    },

    #[serde(rename = "agent:exited")]
    AgentExited {
        agent_id: AgentId,
        exit_code: Option<i32>,
        /// Owner of this agent (job or agent_run).
        owner: OwnerId,
    },

    #[serde(rename = "agent:gone")]
    AgentGone {
        agent_id: AgentId,
        /// Owner of this agent (job or agent_run).
        owner: OwnerId,
    },

    /// User-initiated input to an agent
    #[serde(rename = "agent:input")]
    AgentInput { agent_id: AgentId, input: String },

    #[serde(rename = "agent:signal")]
    AgentSignal {
        agent_id: AgentId,
        /// Kind of signal: "complete" advances job, "escalate" pauses for human
        kind: AgentSignalKind,
        /// Optional message explaining the signal
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Agent is idle (from Notification hook)
    #[serde(rename = "agent:idle")]
    AgentIdle { agent_id: AgentId },

    /// Agent stop hook fired with on_stop=escalate (from CLI hook)
    #[serde(rename = "agent:stop")]
    AgentStop { agent_id: AgentId },

    /// Agent is showing a prompt (from Notification hook)
    #[serde(rename = "agent:prompt")]
    AgentPrompt {
        agent_id: AgentId,
        #[serde(default = "agent::default_prompt_type")]
        prompt_type: PromptType,
        /// Populated when prompt_type is Question — contains the actual question and options
        #[serde(default, skip_serializing_if = "Option::is_none")]
        question_data: Option<QuestionData>,
        /// Last assistant text from the session transcript, providing context for the prompt
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assistant_context: Option<String>,
    },

    // -- command --
    #[serde(rename = "command:run")]
    CommandRun {
        job_id: JobId,
        job_name: String,
        project_root: PathBuf,
        /// Directory where the CLI was invoked (cwd), exposed as {invoke.dir}
        #[serde(default)]
        invoke_dir: PathBuf,
        /// Project namespace
        #[serde(default)]
        namespace: String,
        command: String,
        args: HashMap<String, String>,
    },

    // -- job --
    #[serde(rename = "job:created")]
    JobCreated {
        id: JobId,
        kind: String,
        name: String,
        runbook_hash: String,
        cwd: PathBuf,
        vars: HashMap<String, String>,
        initial_step: String,
        #[serde(default)]
        created_at_epoch_ms: u64,
        #[serde(default)]
        namespace: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cron_name: Option<String>,
    },

    #[serde(rename = "job:advanced")]
    JobAdvanced { id: JobId, step: String },

    #[serde(rename = "job:updated")]
    JobUpdated {
        id: JobId,
        vars: HashMap<String, String>,
    },

    #[serde(rename = "job:resume")]
    JobResume {
        id: JobId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "core_types::is_empty_map")]
        vars: HashMap<String, String>,
        /// Kill the existing session and start fresh (don't use --resume)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        kill: bool,
    },

    #[serde(rename = "job:cancelling")]
    JobCancelling { id: JobId },

    #[serde(rename = "job:cancel")]
    JobCancel { id: JobId },

    #[serde(rename = "job:suspending")]
    JobSuspending { id: JobId },

    #[serde(rename = "job:suspend")]
    JobSuspend { id: JobId },

    #[serde(rename = "job:deleted")]
    JobDeleted { id: JobId },

    // -- runbook --
    #[serde(rename = "runbook:loaded")]
    RunbookLoaded {
        hash: String,
        version: u32,
        runbook: serde_json::Value,
    },

    // -- session --
    #[serde(rename = "session:created")]
    SessionCreated {
        id: SessionId,
        /// Owner of this session (job or agent_run)
        owner: OwnerId,
    },

    #[serde(rename = "session:input")]
    SessionInput { id: SessionId, input: String },

    #[serde(rename = "session:deleted")]
    SessionDeleted { id: SessionId },

    // -- shell --
    #[serde(rename = "shell:exited")]
    ShellExited {
        job_id: JobId,
        step: String,
        exit_code: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stdout: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stderr: Option<String>,
    },

    // -- step --
    /// Step has started running
    #[serde(rename = "step:started")]
    StepStarted {
        job_id: JobId,
        step: String,
        /// Agent ID if this is an agent step (for recovery)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<AgentId>,
        /// Agent name from the runbook definition
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_name: Option<String>,
    },

    /// Step is waiting for human intervention
    #[serde(rename = "step:waiting")]
    StepWaiting {
        job_id: JobId,
        step: String,
        /// Reason for waiting (e.g., gate failure message)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// Decision ID if this waiting state is associated with a decision
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision_id: Option<String>,
    },

    /// Step completed successfully
    #[serde(rename = "step:completed")]
    StepCompleted { job_id: JobId, step: String },

    /// Step failed
    #[serde(rename = "step:failed")]
    StepFailed {
        job_id: JobId,
        step: String,
        error: String,
    },

    // -- system --
    #[serde(rename = "system:shutdown")]
    Shutdown,

    // -- timer --
    #[serde(rename = "timer:start")]
    TimerStart { id: TimerId },

    // -- workspace --
    #[serde(rename = "workspace:created")]
    WorkspaceCreated {
        id: WorkspaceId,
        path: PathBuf,
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<OwnerId>,
        /// "folder" or "worktree"
        #[serde(default)]
        workspace_type: Option<String>,
    },

    #[serde(rename = "workspace:ready")]
    WorkspaceReady { id: WorkspaceId },

    #[serde(rename = "workspace:failed")]
    WorkspaceFailed { id: WorkspaceId, reason: String },

    #[serde(rename = "workspace:deleted")]
    WorkspaceDeleted { id: WorkspaceId },

    #[serde(rename = "workspace:drop")]
    WorkspaceDrop { id: WorkspaceId },

    // -- cron --
    #[serde(rename = "cron:started")]
    CronStarted {
        cron_name: String,
        project_root: PathBuf,
        runbook_hash: String,
        interval: String,
        /// What this cron runs: "job:name" or "agent:name"
        run_target: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "cron:stopped")]
    CronStopped {
        cron_name: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "cron:once")]
    CronOnce {
        cron_name: String,
        /// Set for job targets
        #[serde(default)]
        job_id: JobId,
        #[serde(default)]
        job_name: String,
        #[serde(default)]
        job_kind: String,
        /// Set for agent targets
        #[serde(default)]
        agent_run_id: Option<String>,
        #[serde(default)]
        agent_name: Option<String>,
        project_root: PathBuf,
        runbook_hash: String,
        /// What this cron runs: "job:name" or "agent:name"
        #[serde(default)]
        run_target: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "cron:fired")]
    CronFired {
        cron_name: String,
        #[serde(default)]
        job_id: JobId,
        #[serde(default)]
        agent_run_id: Option<String>,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "cron:deleted")]
    CronDeleted {
        cron_name: String,
        #[serde(default)]
        namespace: String,
    },

    // -- worker --
    #[serde(rename = "worker:started")]
    WorkerStarted {
        worker_name: String,
        project_root: PathBuf,
        runbook_hash: String,
        #[serde(default)]
        queue_name: String,
        #[serde(default)]
        concurrency: u32,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "worker:wake")]
    WorkerWake {
        worker_name: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "worker:poll_complete")]
    WorkerPollComplete {
        worker_name: String,
        items: Vec<serde_json::Value>,
    },

    #[serde(rename = "worker:take_complete")]
    WorkerTakeComplete {
        worker_name: String,
        item_id: String,
        item: serde_json::Value,
        exit_code: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stderr: Option<String>,
    },

    #[serde(rename = "worker:item_dispatched")]
    WorkerItemDispatched {
        worker_name: String,
        item_id: String,
        job_id: JobId,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "worker:stopped")]
    WorkerStopped {
        worker_name: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "worker:resized")]
    WorkerResized {
        worker_name: String,
        concurrency: u32,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "worker:deleted")]
    WorkerDeleted {
        worker_name: String,
        #[serde(default)]
        namespace: String,
    },

    // -- queue --
    #[serde(rename = "queue:pushed")]
    QueuePushed {
        queue_name: String,
        item_id: String,
        data: HashMap<String, String>,
        pushed_at_epoch_ms: u64,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "queue:taken")]
    QueueTaken {
        queue_name: String,
        item_id: String,
        worker_name: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "queue:completed")]
    QueueCompleted {
        queue_name: String,
        item_id: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "queue:failed")]
    QueueFailed {
        queue_name: String,
        item_id: String,
        error: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "queue:dropped")]
    QueueDropped {
        queue_name: String,
        item_id: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "queue:item_retry")]
    QueueItemRetry {
        queue_name: String,
        item_id: String,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "queue:item_dead")]
    QueueItemDead {
        queue_name: String,
        item_id: String,
        #[serde(default)]
        namespace: String,
    },

    // -- decision --
    #[serde(rename = "decision:created")]
    DecisionCreated {
        id: String,
        job_id: JobId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        /// Owner of this decision (job or agent_run).
        owner: OwnerId,
        source: DecisionSource,
        context: String,
        #[serde(default)]
        options: Vec<DecisionOption>,
        /// Structured question data for multi-question decisions
        #[serde(default, skip_serializing_if = "Option::is_none")]
        question_data: Option<QuestionData>,
        created_at_ms: u64,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "decision:resolved")]
    DecisionResolved {
        id: String,
        /// 1-indexed choice picking a numbered option
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chosen: Option<usize>,
        /// Per-question 1-indexed answers for multi-question decisions
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<usize>,
        /// Freeform text (nudge message, custom answer)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        resolved_at_ms: u64,
        #[serde(default)]
        namespace: String,
    },

    // -- agent_run --
    #[serde(rename = "agent_run:created")]
    AgentRunCreated {
        id: AgentRunId,
        agent_name: String,
        command_name: String,
        #[serde(default)]
        namespace: String,
        cwd: PathBuf,
        runbook_hash: String,
        #[serde(default)]
        vars: HashMap<String, String>,
        #[serde(default)]
        created_at_epoch_ms: u64,
    },

    #[serde(rename = "agent_run:started")]
    AgentRunStarted { id: AgentRunId, agent_id: AgentId },

    #[serde(rename = "agent_run:status_changed")]
    AgentRunStatusChanged {
        id: AgentRunId,
        status: AgentRunStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    #[serde(rename = "agent_run:resume")]
    AgentRunResume {
        id: AgentRunId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        kill: bool,
    },

    #[serde(rename = "agent_run:deleted")]
    AgentRunDeleted { id: AgentRunId },

    /// Catch-all for unknown event types (extensibility)
    #[serde(other, skip_serializing)]
    Custom,
}

#[cfg(test)]
#[path = "../event_tests/mod.rs"]
mod tests;
