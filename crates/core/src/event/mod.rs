// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event types for the Odd Jobs system

mod methods;

use crate::agent::{AgentError, AgentId};
use crate::crew::{CrewId, CrewStatus};
use crate::decision::{DecisionId, DecisionOption, DecisionSource};
use crate::job::JobId;
use crate::owner::OwnerId;
use crate::target::RunTarget;
use crate::timer::TimerId;
use crate::workspace::WorkspaceId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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
    #[serde(rename = "system:shutdown")]
    Shutdown,

    #[serde(rename = "timer:start")]
    TimerStart { id: TimerId },

    #[serde(rename = "runbook:loaded")]
    RunbookLoaded { hash: String, version: u32, runbook: serde_json::Value },

    #[serde(rename = "command:run")]
    CommandRun {
        owner: OwnerId,
        name: String,
        project_path: PathBuf,
        /// Directory where the CLI was invoked (cwd), exposed as {invoke.dir}
        invoke_dir: PathBuf,
        project: String,
        command: String,
        args: HashMap<String, String>,
    },

    #[serde(rename = "agent:working")]
    AgentWorking { id: AgentId, owner: OwnerId },

    #[serde(rename = "agent:waiting")]
    AgentWaiting { id: AgentId, owner: OwnerId },

    #[serde(rename = "agent:failed")]
    AgentFailed { id: AgentId, error: AgentError, owner: OwnerId },

    #[serde(rename = "agent:exited")]
    AgentExited {
        id: AgentId,
        owner: OwnerId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },

    #[serde(rename = "agent:gone")]
    AgentGone {
        id: AgentId,
        owner: OwnerId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },

    /// User-initiated input to an agent
    #[serde(rename = "agent:input")]
    AgentInput { id: AgentId, input: String },

    /// Structured response to an agent prompt (plan, permission).
    #[serde(rename = "agent:respond")]
    AgentRespond { id: AgentId, response: crate::agent::PromptResponse },

    /// Agent is idle (from Notification hook)
    #[serde(rename = "agent:idle")]
    AgentIdle { id: AgentId },

    /// Agent stop was blocked by coop's StopConfig (agent tried to exit)
    #[serde(rename = "agent:stop:blocked")]
    AgentStopBlocked { id: AgentId },

    /// Agent stop was allowed by coop's StopConfig (turn ended naturally)
    #[serde(rename = "agent:stop:allowed")]
    AgentStopAllowed { id: AgentId },

    /// Agent is showing a prompt (from Notification hook)
    #[serde(rename = "agent:prompt")]
    AgentPrompt {
        id: AgentId,
        #[serde(default = "default_prompt_type")]
        prompt_type: PromptType,
        /// Populated when prompt_type is Question — contains the actual question and options
        #[serde(default, skip_serializing_if = "Option::is_none")]
        questions: Option<QuestionData>,
        /// Last assistant text from the session transcript, providing context for the prompt
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_message: Option<String>,
    },

    /// Agent spawn completed successfully (background SpawnAgent task finished)
    #[serde(rename = "agent:spawned")]
    AgentSpawned { id: AgentId, owner: OwnerId },

    /// Agent spawn failed (background task couldn't create the session)
    #[serde(rename = "agent:spawn:failed")]
    AgentSpawnFailed { id: AgentId, owner: OwnerId, reason: String },

    #[serde(rename = "crew:created")]
    CrewCreated {
        id: CrewId,
        agent: String,
        project: String,
        command: String,
        cwd: PathBuf,
        vars: HashMap<String, String>,
        runbook_hash: String,
        created_at_ms: u64,
    },

    #[serde(rename = "crew:started")]
    CrewStarted { id: CrewId, agent_id: AgentId },

    #[serde(rename = "crew:updated")]
    CrewUpdated {
        id: CrewId,
        status: CrewStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    #[serde(rename = "crew:resume")]
    CrewResume {
        id: CrewId,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        kill: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    #[serde(rename = "crew:deleted")]
    CrewDeleted { id: CrewId },

    #[serde(rename = "job:created")]
    JobCreated {
        id: JobId,
        kind: String,
        name: String,
        project: String,
        runbook_hash: String,
        cwd: PathBuf,
        vars: HashMap<String, String>,
        initial_step: String,
        created_at_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cron: Option<String>,
    },

    #[serde(rename = "job:advanced")]
    JobAdvanced { id: JobId, step: String },

    #[serde(rename = "job:updated")]
    JobUpdated { id: JobId, vars: HashMap<String, String> },

    #[serde(rename = "job:resume")]
    JobResume {
        id: JobId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "is_empty_map")]
        vars: HashMap<String, String>,
        /// Kill the existing session and start fresh (don't use --resume)
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        kill: bool,
    },

    #[serde(rename = "job:failing")]
    JobFailing { id: JobId },

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

    #[serde(rename = "step:completed")]
    StepCompleted { job_id: JobId, step: String },

    #[serde(rename = "step:failed")]
    StepFailed { job_id: JobId, step: String, error: String },

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

    #[serde(rename = "workspace:created")]
    WorkspaceCreated {
        id: WorkspaceId,
        path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        owner: OwnerId,
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

    #[serde(rename = "cron:started")]
    CronStarted {
        cron: String,
        project: String,
        project_path: PathBuf,
        runbook_hash: String,
        interval: String,
        target: RunTarget,
    },

    #[serde(rename = "cron:stopped")]
    CronStopped { cron: String, project: String },

    #[serde(rename = "cron:once")]
    CronOnce {
        cron: String,
        project: String,
        project_path: PathBuf,
        owner: OwnerId,
        runbook_hash: String,
        target: RunTarget,
    },

    #[serde(rename = "cron:fired")]
    CronFired { cron: String, project: String, owner: OwnerId },

    #[serde(rename = "cron:deleted")]
    CronDeleted { cron: String, project: String },

    // -- worker --
    #[serde(rename = "worker:started")]
    WorkerStarted {
        queue: String,
        worker: String,
        runbook_hash: String,
        concurrency: u32,
        project: String,
        project_path: PathBuf,
    },

    #[serde(rename = "worker:wake")]
    WorkerWake { worker: String, project: String },

    #[serde(rename = "worker:polled")]
    WorkerPolled { worker: String, project: String, items: Vec<serde_json::Value> },

    #[serde(rename = "worker:took")]
    WorkerTook {
        worker: String,
        project: String,
        item_id: String,
        item: serde_json::Value,
        exit_code: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stderr: Option<String>,
    },

    #[serde(rename = "worker:dispatched")]
    WorkerDispatched { worker: String, project: String, owner: OwnerId, item_id: String },

    #[serde(rename = "worker:stopped")]
    WorkerStopped { worker: String, project: String },

    #[serde(rename = "worker:resized")]
    WorkerResized { worker: String, project: String, concurrency: u32 },

    #[serde(rename = "worker:deleted")]
    WorkerDeleted { worker: String, project: String },

    #[serde(rename = "queue:pushed")]
    QueuePushed {
        queue: String,
        project: String,
        item_id: String,
        data: HashMap<String, String>,
        pushed_at_ms: u64,
    },

    #[serde(rename = "queue:taken")]
    QueueTaken { queue: String, project: String, worker: String, item_id: String },

    #[serde(rename = "queue:completed")]
    QueueCompleted { queue: String, project: String, item_id: String },

    #[serde(rename = "queue:failed")]
    QueueFailed { queue: String, project: String, item_id: String, error: String },

    #[serde(rename = "queue:dropped")]
    QueueDropped { queue: String, project: String, item_id: String },

    #[serde(rename = "queue:retry")]
    QueueRetry { queue: String, project: String, item_id: String },

    #[serde(rename = "queue:dead")]
    QueueDead { queue: String, project: String, item_id: String },

    #[serde(rename = "decision:created")]
    DecisionCreated {
        id: DecisionId,
        owner: OwnerId,
        project: String,
        created_at_ms: u64,

        agent_id: AgentId,

        source: DecisionSource,
        context: String,
        #[serde(default)]
        options: Vec<DecisionOption>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        questions: Option<QuestionData>,
    },

    #[serde(rename = "decision:resolved")]
    DecisionResolved {
        id: DecisionId,
        project: String,
        resolved_at_ms: u64,

        /// Per-question 1-indexed answers for multi-question decisions
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        choices: Vec<usize>,
        /// Freeform text (nudge message, custom answer)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Catch-all for unknown event types (extensibility)
    #[serde(other, skip_serializing)]
    Custom,
}
