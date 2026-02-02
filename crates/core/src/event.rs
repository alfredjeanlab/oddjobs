// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event types for the Odd Jobs system

use crate::agent::{AgentError, AgentId, AgentState};
use crate::pipeline::PipelineId;
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
    /// Advance the pipeline to the next step
    Complete,
    /// Pause the pipeline and notify for human intervention
    Escalate,
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
    AgentWorking { agent_id: AgentId },

    #[serde(rename = "agent:waiting")]
    AgentWaiting { agent_id: AgentId },

    #[serde(rename = "agent:failed")]
    AgentFailed {
        agent_id: AgentId,
        error: AgentError,
    },

    #[serde(rename = "agent:exited")]
    AgentExited {
        agent_id: AgentId,
        exit_code: Option<i32>,
    },

    #[serde(rename = "agent:gone")]
    AgentGone { agent_id: AgentId },

    /// User-initiated input to an agent
    #[serde(rename = "agent:input")]
    AgentInput { agent_id: AgentId, input: String },

    #[serde(rename = "agent:signal")]
    AgentSignal {
        agent_id: AgentId,
        /// Kind of signal: "complete" advances pipeline, "escalate" pauses for human
        kind: AgentSignalKind,
        /// Optional message explaining the signal
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    // -- command --
    #[serde(rename = "command:run")]
    CommandRun {
        pipeline_id: PipelineId,
        pipeline_name: String,
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

    // -- pipeline --
    #[serde(rename = "pipeline:created")]
    PipelineCreated {
        id: PipelineId,
        kind: String,
        name: String,
        runbook_hash: String,
        cwd: PathBuf,
        #[serde(alias = "input")]
        vars: HashMap<String, String>,
        initial_step: String,
        #[serde(default)]
        created_at_epoch_ms: u64,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "pipeline:advanced")]
    PipelineAdvanced { id: PipelineId, step: String },

    #[serde(rename = "pipeline:updated")]
    PipelineUpdated {
        id: PipelineId,
        #[serde(alias = "input")]
        vars: HashMap<String, String>,
    },

    #[serde(rename = "pipeline:resume")]
    PipelineResume {
        id: PipelineId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "is_empty_map", alias = "input")]
        vars: HashMap<String, String>,
    },

    #[serde(rename = "pipeline:cancel")]
    PipelineCancel { id: PipelineId },

    #[serde(rename = "pipeline:deleted")]
    PipelineDeleted { id: PipelineId },

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
        pipeline_id: PipelineId,
    },

    #[serde(rename = "session:input")]
    SessionInput { id: SessionId, input: String },

    #[serde(rename = "session:deleted")]
    SessionDeleted { id: SessionId },

    // -- shell --
    #[serde(rename = "shell:exited")]
    ShellExited {
        pipeline_id: PipelineId,
        step: String,
        exit_code: i32,
    },

    // -- step --
    /// Step has started running
    #[serde(rename = "step:started")]
    StepStarted {
        pipeline_id: PipelineId,
        step: String,
        /// Agent ID if this is an agent step (for recovery)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<AgentId>,
    },

    /// Step is waiting for human intervention
    #[serde(rename = "step:waiting")]
    StepWaiting {
        pipeline_id: PipelineId,
        step: String,
        /// Reason for waiting (e.g., gate failure message)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// Step completed successfully
    #[serde(rename = "step:completed")]
    StepCompleted {
        pipeline_id: PipelineId,
        step: String,
    },

    /// Step failed
    #[serde(rename = "step:failed")]
    StepFailed {
        pipeline_id: PipelineId,
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
        mode: Option<String>,
    },

    #[serde(rename = "workspace:ready")]
    WorkspaceReady { id: WorkspaceId },

    #[serde(rename = "workspace:failed")]
    WorkspaceFailed { id: WorkspaceId, reason: String },

    #[serde(rename = "workspace:deleted")]
    WorkspaceDeleted { id: WorkspaceId },

    #[serde(rename = "workspace:drop")]
    WorkspaceDrop { id: WorkspaceId },

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

    #[serde(rename = "worker:item_dispatched")]
    WorkerItemDispatched {
        worker_name: String,
        item_id: String,
        pipeline_id: PipelineId,
        #[serde(default)]
        namespace: String,
    },

    #[serde(rename = "worker:stopped")]
    WorkerStopped {
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

    /// Catch-all for unknown event types (extensibility)
    #[serde(other, skip_serializing)]
    Custom,
}

impl Event {
    /// Create an agent event from an AgentState
    pub fn from_agent_state(agent_id: AgentId, state: AgentState) -> Self {
        match state {
            AgentState::Working => Event::AgentWorking { agent_id },
            AgentState::WaitingForInput => Event::AgentWaiting { agent_id },
            AgentState::Failed(error) => Event::AgentFailed { agent_id, error },
            AgentState::Exited { exit_code } => Event::AgentExited {
                agent_id,
                exit_code,
            },
            AgentState::SessionGone => Event::AgentGone { agent_id },
        }
    }

    /// Extract agent_id and state if this is an agent event
    pub fn as_agent_state(&self) -> Option<(&AgentId, AgentState)> {
        match self {
            Event::AgentWorking { agent_id } => Some((agent_id, AgentState::Working)),
            Event::AgentWaiting { agent_id } => Some((agent_id, AgentState::WaitingForInput)),
            Event::AgentFailed { agent_id, error } => {
                Some((agent_id, AgentState::Failed(error.clone())))
            }
            Event::AgentExited {
                agent_id,
                exit_code,
            } => Some((
                agent_id,
                AgentState::Exited {
                    exit_code: *exit_code,
                },
            )),
            Event::AgentGone { agent_id } => Some((agent_id, AgentState::SessionGone)),
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
            Event::CommandRun { .. } => "command:run",
            Event::PipelineCreated { .. } => "pipeline:created",
            Event::PipelineAdvanced { .. } => "pipeline:advanced",
            Event::PipelineUpdated { .. } => "pipeline:updated",
            Event::PipelineResume { .. } => "pipeline:resume",
            Event::PipelineCancel { .. } => "pipeline:cancel",
            Event::PipelineDeleted { .. } => "pipeline:deleted",
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
            Event::WorkerStarted { .. } => "worker:started",
            Event::WorkerWake { .. } => "worker:wake",
            Event::WorkerPollComplete { .. } => "worker:poll_complete",
            Event::WorkerItemDispatched { .. } => "worker:item_dispatched",
            Event::WorkerStopped { .. } => "worker:stopped",
            Event::QueuePushed { .. } => "queue:pushed",
            Event::QueueTaken { .. } => "queue:taken",
            Event::QueueCompleted { .. } => "queue:completed",
            Event::QueueFailed { .. } => "queue:failed",
            Event::QueueDropped { .. } => "queue:dropped",
            Event::QueueItemRetry { .. } => "queue:item_retry",
            Event::QueueItemDead { .. } => "queue:item_dead",
            Event::Custom => "custom",
        }
    }

    pub fn log_summary(&self) -> String {
        let t = self.name();
        match self {
            Event::AgentWorking { agent_id }
            | Event::AgentWaiting { agent_id }
            | Event::AgentFailed { agent_id, .. }
            | Event::AgentExited { agent_id, .. }
            | Event::AgentGone { agent_id } => format!("{t} agent={agent_id}"),
            Event::AgentInput { agent_id, .. } => format!("{t} agent={agent_id}"),
            Event::AgentSignal { agent_id, kind, .. } => {
                format!("{t} id={agent_id} kind={kind:?}")
            }
            Event::CommandRun {
                pipeline_id,
                command,
                namespace,
                ..
            } => {
                if namespace.is_empty() {
                    format!("{t} id={pipeline_id} cmd={command}")
                } else {
                    format!("{t} id={pipeline_id} ns={namespace} cmd={command}")
                }
            }
            Event::PipelineCreated {
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
            Event::PipelineAdvanced { id, step } => format!("{t} id={id} step={step}"),
            Event::PipelineUpdated { id, .. } => format!("{t} id={id}"),
            Event::PipelineResume { id, .. } => format!("{t} id={id}"),
            Event::PipelineCancel { id } => format!("{t} id={id}"),
            Event::PipelineDeleted { id } => format!("{t} id={id}"),
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
                let pipelines = runbook
                    .get("pipelines")
                    .and_then(|v| v.as_object())
                    .map(|o| o.len())
                    .unwrap_or(0);
                format!(
                    "{t} hash={} v={version} agents={agents} pipelines={pipelines}",
                    &hash[..12]
                )
            }
            Event::SessionCreated { id, pipeline_id } => {
                format!("{t} id={id} pipeline={pipeline_id}")
            }
            Event::SessionInput { id, .. } => format!("{t} id={id}"),
            Event::SessionDeleted { id } => format!("{t} id={id}"),
            Event::ShellExited {
                pipeline_id,
                step,
                exit_code,
            } => format!("{t} pipeline={pipeline_id} step={step} exit={exit_code}"),
            Event::StepStarted {
                pipeline_id, step, ..
            } => format!("{t} pipeline={pipeline_id} step={step}"),
            Event::StepWaiting {
                pipeline_id, step, ..
            } => format!("{t} pipeline={pipeline_id} step={step}"),
            Event::StepCompleted { pipeline_id, step } => {
                format!("{t} pipeline={pipeline_id} step={step}")
            }
            Event::StepFailed {
                pipeline_id, step, ..
            } => format!("{t} pipeline={pipeline_id} step={step}"),
            Event::Shutdown | Event::Custom => t.to_string(),
            Event::TimerStart { id } => format!("{t} id={id}"),
            Event::WorkspaceCreated { id, .. } => format!("{t} id={id}"),
            Event::WorkspaceReady { id }
            | Event::WorkspaceFailed { id, .. }
            | Event::WorkspaceDeleted { id } => format!("{t} id={id}"),
            Event::WorkspaceDrop { id, .. } => format!("{t} id={id}"),
            Event::WorkerStarted { worker_name, .. } => {
                format!("{t} worker={worker_name}")
            }
            Event::WorkerWake { worker_name, .. } => format!("{t} worker={worker_name}"),
            Event::WorkerPollComplete {
                worker_name, items, ..
            } => format!("{t} worker={worker_name} items={}", items.len()),
            Event::WorkerItemDispatched {
                worker_name,
                item_id,
                pipeline_id,
                ..
            } => format!("{t} worker={worker_name} item={item_id} pipeline={pipeline_id}"),
            Event::WorkerStopped { worker_name, .. } => format!("{t} worker={worker_name}"),
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
        }
    }

    pub fn pipeline_id(&self) -> Option<&PipelineId> {
        match self {
            Event::CommandRun { pipeline_id, .. }
            | Event::SessionCreated { pipeline_id, .. }
            | Event::ShellExited { pipeline_id, .. }
            | Event::StepStarted { pipeline_id, .. }
            | Event::StepWaiting { pipeline_id, .. }
            | Event::StepCompleted { pipeline_id, .. }
            | Event::StepFailed { pipeline_id, .. } => Some(pipeline_id),
            Event::PipelineCreated { id, .. }
            | Event::PipelineAdvanced { id, .. }
            | Event::PipelineUpdated { id, .. }
            | Event::PipelineResume { id, .. }
            | Event::PipelineCancel { id, .. }
            | Event::PipelineDeleted { id, .. } => Some(id),
            Event::WorkerItemDispatched { pipeline_id, .. } => Some(pipeline_id),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
