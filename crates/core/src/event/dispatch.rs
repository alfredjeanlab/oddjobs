// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event dispatch methods — name, log summary, job_id extraction

use super::{
    agent, agent_run, core_types, cron_scheduler, decision, job, step, worker_queue, workspace,
    Event,
};
use crate::agent::{AgentId, AgentState};
use crate::job::JobId;
use crate::owner::OwnerId;

impl Event {
    /// Create an agent event from an AgentState with owner.
    pub fn from_agent_state(agent_id: AgentId, state: AgentState, owner: OwnerId) -> Self {
        agent::from_agent_state(agent_id, state, owner)
    }

    /// Extract agent_id, state, and owner if this is an agent event.
    pub fn as_agent_state(&self) -> Option<(&AgentId, AgentState, &OwnerId)> {
        agent::as_agent_state(self)
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
            Event::JobSuspending { .. } => "job:suspending",
            Event::JobSuspend { .. } => "job:suspend",
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
            Event::WorkerResized { .. } => "worker:resized",
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
            Event::AgentRunResume { .. } => "agent_run:resume",
            Event::AgentRunDeleted { .. } => "agent_run:deleted",
            Event::Custom => "custom",
        }
    }

    pub fn log_summary(&self) -> String {
        let t = self.name();
        match self {
            // Agent events
            Event::AgentWorking { .. }
            | Event::AgentWaiting { .. }
            | Event::AgentFailed { .. }
            | Event::AgentExited { .. }
            | Event::AgentGone { .. }
            | Event::AgentInput { .. }
            | Event::AgentSignal { .. }
            | Event::AgentIdle { .. }
            | Event::AgentStop { .. }
            | Event::AgentPrompt { .. } => agent::log_summary(self, t),

            // Job events
            Event::JobCreated { .. }
            | Event::JobAdvanced { .. }
            | Event::JobUpdated { .. }
            | Event::JobResume { .. }
            | Event::JobCancelling { .. }
            | Event::JobCancel { .. }
            | Event::JobSuspending { .. }
            | Event::JobSuspend { .. }
            | Event::JobDeleted { .. } => job::log_summary(self, t),

            // Step events
            Event::StepStarted { .. }
            | Event::StepWaiting { .. }
            | Event::StepCompleted { .. }
            | Event::StepFailed { .. } => step::log_summary(self, t),

            // Workspace events
            Event::WorkspaceCreated { .. }
            | Event::WorkspaceReady { .. }
            | Event::WorkspaceFailed { .. }
            | Event::WorkspaceDeleted { .. }
            | Event::WorkspaceDrop { .. } => workspace::log_summary(self, t),

            // Worker and queue events
            Event::WorkerStarted { .. }
            | Event::WorkerWake { .. }
            | Event::WorkerPollComplete { .. }
            | Event::WorkerTakeComplete { .. }
            | Event::WorkerItemDispatched { .. }
            | Event::WorkerStopped { .. }
            | Event::WorkerResized { .. }
            | Event::WorkerDeleted { .. }
            | Event::QueuePushed { .. }
            | Event::QueueTaken { .. }
            | Event::QueueCompleted { .. }
            | Event::QueueFailed { .. }
            | Event::QueueDropped { .. }
            | Event::QueueItemRetry { .. }
            | Event::QueueItemDead { .. } => worker_queue::log_summary(self, t),

            // Cron events
            Event::CronStarted { .. }
            | Event::CronStopped { .. }
            | Event::CronOnce { .. }
            | Event::CronFired { .. }
            | Event::CronDeleted { .. } => cron_scheduler::log_summary(self, t),

            // Decision events
            Event::DecisionCreated { .. } | Event::DecisionResolved { .. } => {
                decision::log_summary(self, t)
            }

            // Agent run events
            Event::AgentRunCreated { .. }
            | Event::AgentRunStarted { .. }
            | Event::AgentRunStatusChanged { .. }
            | Event::AgentRunResume { .. }
            | Event::AgentRunDeleted { .. } => agent_run::log_summary(self, t),

            // Core events (command, runbook, session, shell, system, timer)
            Event::CommandRun { .. }
            | Event::RunbookLoaded { .. }
            | Event::SessionCreated { .. }
            | Event::SessionInput { .. }
            | Event::SessionDeleted { .. }
            | Event::ShellExited { .. }
            | Event::Shutdown
            | Event::TimerStart { .. }
            | Event::Custom => core_types::log_summary(self, t),
        }
    }

    pub fn job_id(&self) -> Option<&JobId> {
        match self {
            // Job events
            Event::JobCreated { .. }
            | Event::JobAdvanced { .. }
            | Event::JobUpdated { .. }
            | Event::JobResume { .. }
            | Event::JobCancelling { .. }
            | Event::JobCancel { .. }
            | Event::JobSuspending { .. }
            | Event::JobSuspend { .. }
            | Event::JobDeleted { .. } => job::job_id(self),

            // Step events
            Event::StepStarted { .. }
            | Event::StepWaiting { .. }
            | Event::StepCompleted { .. }
            | Event::StepFailed { .. } => step::job_id(self),

            // Worker dispatch
            Event::WorkerItemDispatched { .. } => worker_queue::job_id(self),

            // Cron events
            Event::CronOnce { .. } | Event::CronFired { .. } => cron_scheduler::job_id(self),

            // Decision events
            Event::DecisionCreated { .. } => decision::job_id(self),

            // Core events (command, shell, session)
            Event::CommandRun { .. } | Event::ShellExited { .. } | Event::SessionCreated { .. } => {
                core_types::job_id(self)
            }

            _ => None,
        }
    }
}
