// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event types for the Odd Jobs system

use crate::agent::{AgentError, AgentId, AgentState};
use crate::agent_run::{AgentRunId, AgentRunStatus};
use crate::decision::{DecisionOption, DecisionSource};
use crate::id::ShortId;
use crate::job::JobId;
use crate::owner::OwnerId;
use crate::session::SessionId;
use crate::timer::TimerId;
use crate::workspace::WorkspaceId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Kind of signal an agent can emit
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSignalKind {
    /// Advance the job to the next step
    Complete,
    /// Pause the job and notify for human intervention
    Escalate,
    /// No-op acknowledgement — agent is still working
    Continue,
}

/// Type of prompt the agent is showing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptType {
    Permission,
    Idle,
    PlanApproval,
    Question,
    Other,
}

/// Structured data from an AskUserQuestion tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionData {
    pub questions: Vec<QuestionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionEntry {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default, rename = "multiSelect")]
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

fn default_prompt_type() -> PromptType {
    PromptType::Other
}

fn is_empty_map<K, V>(map: &HashMap<K, V>) -> bool {
    map.is_empty()
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
        /// Owner of this agent (job or agent_run). None for legacy events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<OwnerId>,
    },

    #[serde(rename = "agent:waiting")]
    AgentWaiting {
        agent_id: AgentId,
        /// Owner of this agent (job or agent_run). None for legacy events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<OwnerId>,
    },

    #[serde(rename = "agent:failed")]
    AgentFailed {
        agent_id: AgentId,
        error: AgentError,
        /// Owner of this agent (job or agent_run). None for legacy events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<OwnerId>,
    },

    #[serde(rename = "agent:exited")]
    AgentExited {
        agent_id: AgentId,
        exit_code: Option<i32>,
        /// Owner of this agent (job or agent_run). None for legacy events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<OwnerId>,
    },

    #[serde(rename = "agent:gone")]
    AgentGone {
        agent_id: AgentId,
        /// Owner of this agent (job or agent_run). None for legacy events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<OwnerId>,
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
        #[serde(default = "default_prompt_type")]
        prompt_type: PromptType,
        /// Populated when prompt_type is Question — contains the actual question and options
        #[serde(default, skip_serializing_if = "Option::is_none")]
        question_data: Option<QuestionData>,
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
        #[serde(default, skip_serializing_if = "is_empty_map")]
        vars: HashMap<String, String>,
    },

    #[serde(rename = "job:cancelling")]
    JobCancelling { id: JobId },

    #[serde(rename = "job:cancel")]
    JobCancel { id: JobId },

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
        job_id: JobId,
        /// For standalone agents, the AgentRunId that owns this session
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_run_id: Option<AgentRunId>,
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
        owner: Option<String>,
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
        source: DecisionSource,
        context: String,
        #[serde(default)]
        options: Vec<DecisionOption>,
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

    #[serde(rename = "agent_run:deleted")]
    AgentRunDeleted { id: AgentRunId },

    /// Catch-all for unknown event types (extensibility)
    #[serde(other, skip_serializing)]
    Custom,
}

impl Event {
    /// Create an agent event from an AgentState with optional owner.
    pub fn from_agent_state(agent_id: AgentId, state: AgentState, owner: Option<OwnerId>) -> Self {
        match state {
            AgentState::Working => Event::AgentWorking { agent_id, owner },
            AgentState::WaitingForInput => Event::AgentWaiting { agent_id, owner },
            AgentState::Failed(error) => Event::AgentFailed {
                agent_id,
                error,
                owner,
            },
            AgentState::Exited { exit_code } => Event::AgentExited {
                agent_id,
                exit_code,
                owner,
            },
            AgentState::SessionGone => Event::AgentGone { agent_id, owner },
        }
    }

    /// Extract agent_id, state, and owner if this is an agent event.
    pub fn as_agent_state(&self) -> Option<(&AgentId, AgentState, Option<&OwnerId>)> {
        match self {
            Event::AgentWorking { agent_id, owner } => {
                Some((agent_id, AgentState::Working, owner.as_ref()))
            }
            Event::AgentWaiting { agent_id, owner } => {
                Some((agent_id, AgentState::WaitingForInput, owner.as_ref()))
            }
            Event::AgentFailed {
                agent_id,
                error,
                owner,
            } => Some((agent_id, AgentState::Failed(error.clone()), owner.as_ref())),
            Event::AgentExited {
                agent_id,
                exit_code,
                owner,
            } => Some((
                agent_id,
                AgentState::Exited {
                    exit_code: *exit_code,
                },
                owner.as_ref(),
            )),
            Event::AgentGone { agent_id, owner } => {
                Some((agent_id, AgentState::SessionGone, owner.as_ref()))
            }
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Event::AgentWorking { .. } => "agent:working",
            Event::AgentWaiting { .. } => "agent:waiting",
            Event::AgentFailed { .. } => "agent:failed",
            Event::AgentExited { .. } => "agent:exited",
            Event::AgentGone { .. } => "agent:gone",
            Event::AgentInput { .. } => "agent:input",
            Event::AgentSignal { .. } => "agent:signal",
            Event::AgentIdle { .. } => "agent:idle",
            Event::AgentStop { .. } => "agent:stop",
            Event::AgentPrompt { .. } => "agent:prompt",
            Event::CommandRun { .. } => "command:run",
            Event::JobCreated { .. } => "job:created",
            Event::JobAdvanced { .. } => "job:advanced",
            Event::JobUpdated { .. } => "job:updated",
            Event::JobResume { .. } => "job:resume",
            Event::JobCancelling { .. } => "job:cancelling",
            Event::JobCancel { .. } => "job:cancel",
            Event::JobDeleted { .. } => "job:deleted",
            Event::RunbookLoaded { .. } => "runbook:loaded",
            Event::SessionCreated { .. } => "session:created",
            Event::SessionInput { .. } => "session:input",
            Event::SessionDeleted { .. } => "session:deleted",
            Event::ShellExited { .. } => "shell:exited",
            Event::StepStarted { .. } => "step:started",
            Event::StepWaiting { .. } => "step:waiting",
            Event::StepCompleted { .. } => "step:completed",
            Event::StepFailed { .. } => "step:failed",
            Event::Shutdown => "system:shutdown",
            Event::TimerStart { .. } => "timer:start",
            Event::WorkspaceCreated { .. } => "workspace:created",
            Event::WorkspaceReady { .. } => "workspace:ready",
            Event::WorkspaceFailed { .. } => "workspace:failed",
            Event::WorkspaceDeleted { .. } => "workspace:deleted",
            Event::WorkspaceDrop { .. } => "workspace:drop",
            Event::CronStarted { .. } => "cron:started",
            Event::CronStopped { .. } => "cron:stopped",
            Event::CronOnce { .. } => "cron:once",
            Event::CronFired { .. } => "cron:fired",
            Event::CronDeleted { .. } => "cron:deleted",
            Event::WorkerStarted { .. } => "worker:started",
            Event::WorkerWake { .. } => "worker:wake",
            Event::WorkerPollComplete { .. } => "worker:poll_complete",
            Event::WorkerTakeComplete { .. } => "worker:take_complete",
            Event::WorkerItemDispatched { .. } => "worker:item_dispatched",
            Event::WorkerStopped { .. } => "worker:stopped",
            Event::WorkerDeleted { .. } => "worker:deleted",
            Event::QueuePushed { .. } => "queue:pushed",
            Event::QueueTaken { .. } => "queue:taken",
            Event::QueueCompleted { .. } => "queue:completed",
            Event::QueueFailed { .. } => "queue:failed",
            Event::QueueDropped { .. } => "queue:dropped",
            Event::QueueItemRetry { .. } => "queue:item_retry",
            Event::QueueItemDead { .. } => "queue:item_dead",
            Event::DecisionCreated { .. } => "decision:created",
            Event::DecisionResolved { .. } => "decision:resolved",
            Event::AgentRunCreated { .. } => "agent_run:created",
            Event::AgentRunStarted { .. } => "agent_run:started",
            Event::AgentRunStatusChanged { .. } => "agent_run:status_changed",
            Event::AgentRunDeleted { .. } => "agent_run:deleted",
            Event::Custom => "custom",
        }
    }

    pub fn log_summary(&self) -> String {
        let t = self.name();
        match self {
            Event::AgentWorking { agent_id, .. }
            | Event::AgentWaiting { agent_id, .. }
            | Event::AgentFailed { agent_id, .. }
            | Event::AgentExited { agent_id, .. }
            | Event::AgentGone { agent_id, .. } => format!("{t} agent={agent_id}"),
            Event::AgentInput { agent_id, .. } => format!("{t} agent={agent_id}"),
            Event::AgentSignal { agent_id, kind, .. } => {
                format!("{t} id={agent_id} kind={kind:?}")
            }
            Event::AgentIdle { agent_id } => format!("{t} agent={agent_id}"),
            Event::AgentStop { agent_id } => format!("{t} agent={agent_id}"),
            Event::AgentPrompt {
                agent_id,
                prompt_type,
                ..
            } => format!("{t} agent={agent_id} prompt_type={prompt_type:?}"),
            Event::CommandRun {
                job_id,
                command,
                namespace,
                ..
            } => {
                if namespace.is_empty() {
                    format!("{t} id={job_id} cmd={command}")
                } else {
                    format!("{t} id={job_id} ns={namespace} cmd={command}")
                }
            }
            Event::JobCreated {
                id,
                kind,
                name,
                namespace,
                ..
            } => {
                if namespace.is_empty() {
                    format!("{t} id={id} kind={kind} name={name}")
                } else {
                    format!("{t} id={id} ns={namespace} kind={kind} name={name}")
                }
            }
            Event::JobAdvanced { id, step } => format!("{t} id={id} step={step}"),
            Event::JobUpdated { id, .. } => format!("{t} id={id}"),
            Event::JobResume { id, .. } => format!("{t} id={id}"),
            Event::JobCancelling { id } => format!("{t} id={id}"),
            Event::JobCancel { id } => format!("{t} id={id}"),
            Event::JobDeleted { id } => format!("{t} id={id}"),
            Event::RunbookLoaded {
                hash,
                version,
                runbook,
            } => {
                let agents = runbook
                    .get("agents")
                    .and_then(|v| v.as_object())
                    .map(|o| o.len())
                    .unwrap_or(0);
                let jobs = runbook
                    .get("jobs")
                    .and_then(|v| v.as_object())
                    .map(|o| o.len())
                    .unwrap_or(0);
                format!(
                    "{t} hash={} v={version} agents={agents} jobs={jobs}",
                    hash.short(12)
                )
            }
            Event::SessionCreated {
                id,
                job_id,
                agent_run_id,
            } => {
                if let Some(ref ar_id) = agent_run_id {
                    format!("{t} id={id} agent_run={ar_id}")
                } else {
                    format!("{t} id={id} job={job_id}")
                }
            }
            Event::SessionInput { id, .. } => format!("{t} id={id}"),
            Event::SessionDeleted { id } => format!("{t} id={id}"),
            Event::ShellExited {
                job_id,
                step,
                exit_code,
                ..
            } => format!("{t} job={job_id} step={step} exit={exit_code}"),
            Event::StepStarted { job_id, step, .. } => format!("{t} job={job_id} step={step}"),
            Event::StepWaiting { job_id, step, .. } => format!("{t} job={job_id} step={step}"),
            Event::StepCompleted { job_id, step } => {
                format!("{t} job={job_id} step={step}")
            }
            Event::StepFailed { job_id, step, .. } => format!("{t} job={job_id} step={step}"),
            Event::Shutdown | Event::Custom => t.to_string(),
            Event::TimerStart { id } => format!("{t} id={id}"),
            Event::WorkspaceCreated { id, .. } => format!("{t} id={id}"),
            Event::WorkspaceReady { id }
            | Event::WorkspaceFailed { id, .. }
            | Event::WorkspaceDeleted { id } => format!("{t} id={id}"),
            Event::WorkspaceDrop { id, .. } => format!("{t} id={id}"),
            Event::CronStarted { cron_name, .. } => format!("{t} cron={cron_name}"),
            Event::CronStopped { cron_name, .. } => format!("{t} cron={cron_name}"),
            Event::CronOnce {
                cron_name,
                job_id,
                agent_name,
                ..
            } => {
                if let Some(agent) = agent_name {
                    format!("{t} cron={cron_name} agent={agent}")
                } else {
                    format!("{t} cron={cron_name} job={job_id}")
                }
            }
            Event::CronFired {
                cron_name,
                job_id,
                agent_run_id,
                ..
            } => {
                if let Some(ar_id) = agent_run_id {
                    format!("{t} cron={cron_name} agent_run={ar_id}")
                } else {
                    format!("{t} cron={cron_name} job={job_id}")
                }
            }
            Event::CronDeleted {
                cron_name,
                namespace,
            } => {
                if namespace.is_empty() {
                    format!("{t} cron={cron_name}")
                } else {
                    format!("{t} cron={cron_name} ns={namespace}")
                }
            }
            Event::WorkerStarted { worker_name, .. } => {
                format!("{t} worker={worker_name}")
            }
            Event::WorkerWake { worker_name, .. } => format!("{t} worker={worker_name}"),
            Event::WorkerPollComplete {
                worker_name, items, ..
            } => format!("{t} worker={worker_name} items={}", items.len()),
            Event::WorkerTakeComplete {
                worker_name,
                item_id,
                exit_code,
                ..
            } => format!("{t} worker={worker_name} item={item_id} exit={exit_code}"),
            Event::WorkerItemDispatched {
                worker_name,
                item_id,
                job_id,
                ..
            } => format!("{t} worker={worker_name} item={item_id} job={job_id}"),
            Event::WorkerStopped { worker_name, .. } => format!("{t} worker={worker_name}"),
            Event::WorkerDeleted {
                worker_name,
                namespace,
            } => {
                if namespace.is_empty() {
                    format!("{t} worker={worker_name}")
                } else {
                    format!("{t} worker={worker_name} ns={namespace}")
                }
            }
            Event::QueuePushed {
                queue_name,
                item_id,
                ..
            } => format!("{t} queue={queue_name} item={item_id}"),
            Event::QueueTaken {
                queue_name,
                item_id,
                ..
            } => format!("{t} queue={queue_name} item={item_id}"),
            Event::QueueCompleted {
                queue_name,
                item_id,
                ..
            } => format!("{t} queue={queue_name} item={item_id}"),
            Event::QueueFailed {
                queue_name,
                item_id,
                ..
            } => format!("{t} queue={queue_name} item={item_id}"),
            Event::QueueDropped {
                queue_name,
                item_id,
                ..
            } => format!("{t} queue={queue_name} item={item_id}"),
            Event::QueueItemRetry {
                queue_name,
                item_id,
                ..
            } => format!("{t} queue={queue_name} item={item_id}"),
            Event::QueueItemDead {
                queue_name,
                item_id,
                ..
            } => format!("{t} queue={queue_name} item={item_id}"),
            Event::DecisionCreated {
                id, job_id, source, ..
            } => format!("{t} id={id} job={job_id} source={source:?}"),
            Event::DecisionResolved { id, chosen, .. } => {
                if let Some(c) = chosen {
                    format!("{t} id={id} chosen={c}")
                } else {
                    format!("{t} id={id}")
                }
            }
            Event::AgentRunCreated {
                id,
                agent_name,
                namespace,
                ..
            } => {
                if namespace.is_empty() {
                    format!("{t} id={id} agent={agent_name}")
                } else {
                    format!("{t} id={id} ns={namespace} agent={agent_name}")
                }
            }
            Event::AgentRunStarted { id, agent_id } => {
                format!("{t} id={id} agent_id={agent_id}")
            }
            Event::AgentRunStatusChanged { id, status, reason } => {
                if let Some(reason) = reason {
                    format!("{t} id={id} status={status} reason={reason}")
                } else {
                    format!("{t} id={id} status={status}")
                }
            }
            Event::AgentRunDeleted { id } => format!("{t} id={id}"),
        }
    }

    pub fn job_id(&self) -> Option<&JobId> {
        match self {
            Event::CommandRun { job_id, .. }
            | Event::SessionCreated { job_id, .. }
            | Event::ShellExited { job_id, .. }
            | Event::StepStarted { job_id, .. }
            | Event::StepWaiting { job_id, .. }
            | Event::StepCompleted { job_id, .. }
            | Event::StepFailed { job_id, .. } => Some(job_id),
            Event::JobCreated { id, .. }
            | Event::JobAdvanced { id, .. }
            | Event::JobUpdated { id, .. }
            | Event::JobResume { id, .. }
            | Event::JobCancelling { id, .. }
            | Event::JobCancel { id, .. }
            | Event::JobDeleted { id, .. } => Some(id),
            Event::WorkerItemDispatched { job_id, .. } => Some(job_id),
            Event::CronOnce {
                job_id, agent_name, ..
            } => {
                if agent_name.is_some() {
                    None
                } else {
                    Some(job_id)
                }
            }
            Event::CronFired {
                job_id,
                agent_run_id,
                ..
            } => {
                if agent_run_id.is_some() {
                    None
                } else {
                    Some(job_id)
                }
            }
            Event::DecisionCreated { job_id, .. } => Some(job_id),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
