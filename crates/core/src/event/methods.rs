// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event methods — name, log summary, job_id, agent state conversion

use super::Event;
use crate::agent::{AgentId, AgentState};
use crate::id;
use crate::job::JobId;
use crate::owner::OwnerId;

/// Returns ` ns={project}` when non-empty, empty string otherwise.
fn ns_fragment(project: &str) -> String {
    if project.is_empty() {
        String::new()
    } else {
        format!(" ns={project}")
    }
}

impl Event {
    /// Create an agent event from an AgentState with owner.
    pub fn from_agent_state(id: AgentId, state: AgentState, owner: OwnerId) -> Self {
        match state {
            AgentState::Working => Event::AgentWorking { id, owner },
            AgentState::WaitingForInput => Event::AgentWaiting { id, owner },
            AgentState::Failed(error) => Event::AgentFailed { id, error, owner },
            AgentState::Exited { exit_code } => Event::AgentExited { id, exit_code, owner },
            AgentState::SessionGone => Event::AgentGone { id, owner, exit_code: None },
        }
    }

    /// Extract agent_id, state, and owner if this is an agent event.
    pub fn as_agent_state(&self) -> Option<(&AgentId, AgentState, &OwnerId)> {
        match self {
            Event::AgentWorking { id, owner } => Some((id, AgentState::Working, owner)),
            Event::AgentWaiting { id, owner } => Some((id, AgentState::WaitingForInput, owner)),
            Event::AgentFailed { id, error, owner } => {
                Some((id, AgentState::Failed(error.clone()), owner))
            }
            Event::AgentExited { id, exit_code, owner } => {
                Some((id, AgentState::Exited { exit_code: *exit_code }, owner))
            }
            Event::AgentGone { id, owner, .. } => Some((id, AgentState::SessionGone, owner)),
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
            Event::AgentRespond { .. } => "agent:respond",
            Event::AgentIdle { .. } => "agent:idle",
            Event::AgentStopBlocked { .. } => "agent:stop:blocked",
            Event::AgentStopAllowed { .. } => "agent:stop:allowed",
            Event::AgentPrompt { .. } => "agent:prompt",
            Event::CommandRun { .. } => "command:run",
            Event::JobCreated { .. } => "job:created",
            Event::JobAdvanced { .. } => "job:advanced",
            Event::JobUpdated { .. } => "job:updated",
            Event::JobResume { .. } => "job:resume",
            Event::JobFailing { .. } => "job:failing",
            Event::JobCancelling { .. } => "job:cancelling",
            Event::JobCancel { .. } => "job:cancel",
            Event::JobSuspending { .. } => "job:suspending",
            Event::JobSuspend { .. } => "job:suspend",
            Event::JobDeleted { .. } => "job:deleted",
            Event::RunbookLoaded { .. } => "runbook:loaded",
            Event::AgentSpawned { .. } => "agent:spawned",
            Event::AgentSpawnFailed { .. } => "agent:spawn:failed",
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
            Event::WorkerPolled { .. } => "worker:polled",
            Event::WorkerTook { .. } => "worker:took",
            Event::WorkerDispatched { .. } => "worker:dispatched",
            Event::WorkerStopped { .. } => "worker:stopped",
            Event::WorkerResized { .. } => "worker:resized",
            Event::WorkerDeleted { .. } => "worker:deleted",
            Event::QueuePushed { .. } => "queue:pushed",
            Event::QueueTaken { .. } => "queue:taken",
            Event::QueueCompleted { .. } => "queue:completed",
            Event::QueueFailed { .. } => "queue:failed",
            Event::QueueDropped { .. } => "queue:dropped",
            Event::QueueRetry { .. } => "queue:retry",
            Event::QueueDead { .. } => "queue:dead",
            Event::DecisionCreated { .. } => "decision:created",
            Event::DecisionResolved { .. } => "decision:resolved",
            Event::CrewCreated { .. } => "crew:created",
            Event::CrewStarted { .. } => "crew:started",
            Event::CrewUpdated { .. } => "crew:updated",
            Event::CrewResume { .. } => "crew:resume",
            Event::CrewDeleted { .. } => "crew:deleted",
            Event::Custom => "custom",
        }
    }

    pub fn log_summary(&self) -> String {
        let t = self.name();
        match self {
            // -- agent --
            Event::AgentWorking { id, .. }
            | Event::AgentWaiting { id, .. }
            | Event::AgentFailed { id, .. }
            | Event::AgentExited { id, .. }
            | Event::AgentGone { id, .. }
            | Event::AgentInput { id, .. }
            | Event::AgentRespond { id, .. }
            | Event::AgentIdle { id, .. }
            | Event::AgentStopBlocked { id, .. }
            | Event::AgentStopAllowed { id, .. } => format!("{t} agent={id}"),
            Event::AgentPrompt { id, prompt_type, .. } => {
                format!("{t} agent={id} prompt_type={prompt_type:?}")
            }
            Event::AgentSpawned { id, owner } => {
                format!("{t} agent={id} owner={owner}")
            }
            Event::AgentSpawnFailed { id, reason, .. } => {
                format!("{t} agent={id} reason={reason}")
            }

            // -- command --
            Event::CommandRun { owner, command, project, .. } => {
                format!("{t} {}{} cmd={command}", owner.log(), ns_fragment(project))
            }

            // -- job --
            Event::JobCreated { id, kind, name, project, .. } => {
                format!("{t} id={id}{} kind={kind} name={name}", ns_fragment(project))
            }
            Event::JobAdvanced { id, step } => format!("{t} id={id} step={step}"),
            Event::JobUpdated { id, .. } => format!("{t} id={id}"),
            Event::JobResume { id, .. } => format!("{t} id={id}"),
            Event::JobFailing { id } => format!("{t} id={id}"),
            Event::JobCancelling { id } => format!("{t} id={id}"),
            Event::JobCancel { id } => format!("{t} id={id}"),
            Event::JobSuspending { id } => format!("{t} id={id}"),
            Event::JobSuspend { id } => format!("{t} id={id}"),
            Event::JobDeleted { id } => format!("{t} id={id}"),

            // -- runbook --
            Event::RunbookLoaded { hash, version, runbook } => {
                let agents =
                    runbook.get("agents").and_then(|v| v.as_object()).map(|o| o.len()).unwrap_or(0);
                let jobs =
                    runbook.get("jobs").and_then(|v| v.as_object()).map(|o| o.len()).unwrap_or(0);
                format!("{t} hash={} v={version} agents={agents} jobs={jobs}", id::short(hash, 12))
            }

            // -- shell --
            Event::ShellExited { job_id, step, exit_code, .. } => {
                format!("{t} job={job_id} step={step} exit={exit_code}")
            }

            // -- step --
            Event::StepStarted { job_id, step, .. }
            | Event::StepWaiting { job_id, step, .. }
            | Event::StepCompleted { job_id, step }
            | Event::StepFailed { job_id, step, .. } => format!("{t} job={job_id} step={step}"),

            // -- system / timer --
            Event::Shutdown | Event::Custom => t.to_string(),
            Event::TimerStart { id } => format!("{t} id={id}"),

            // -- workspace --
            Event::WorkspaceCreated { id, .. }
            | Event::WorkspaceReady { id }
            | Event::WorkspaceFailed { id, .. }
            | Event::WorkspaceDeleted { id }
            | Event::WorkspaceDrop { id } => format!("{t} id={id}"),

            // -- cron --
            Event::CronStarted { cron, .. } | Event::CronStopped { cron, .. } => {
                format!("{t} cron={cron}")
            }
            Event::CronOnce { cron, target, .. } => {
                format!("{t} cron={cron} {}", target.log())
            }
            Event::CronFired { cron, owner, .. } => {
                format!("{t} cron={cron} {}", owner.log())
            }
            Event::CronDeleted { cron, project } => {
                format!("{t} cron={cron}{}", ns_fragment(project))
            }

            // -- worker --
            Event::WorkerStarted { worker, .. }
            | Event::WorkerWake { worker, .. }
            | Event::WorkerStopped { worker, .. } => {
                format!("{t} worker={worker}")
            }
            Event::WorkerPolled { worker, project, items, .. } => {
                format!("{t} worker={worker}{} items={}", ns_fragment(project), items.len())
            }
            Event::WorkerTook { worker, project, item_id, exit_code, .. } => {
                format!(
                    "{t} worker={worker}{} item={item_id} exit={exit_code}",
                    ns_fragment(project)
                )
            }
            Event::WorkerDispatched { worker, item_id, owner, .. } => {
                format!("{t} worker={worker} item={item_id} owner={owner}")
            }
            Event::WorkerResized { worker, concurrency, project } => {
                format!("{t} worker={worker}{} concurrency={concurrency}", ns_fragment(project))
            }
            Event::WorkerDeleted { worker, project } => {
                format!("{t} worker={worker}{}", ns_fragment(project))
            }

            Event::QueuePushed { queue, item_id, .. }
            | Event::QueueTaken { queue, item_id, .. }
            | Event::QueueCompleted { queue, item_id, .. }
            | Event::QueueFailed { queue, item_id, .. }
            | Event::QueueDropped { queue, item_id, .. }
            | Event::QueueRetry { queue, item_id, .. }
            | Event::QueueDead { queue, item_id, .. } => {
                format!("{t} queue={queue} item={item_id}")
            }

            Event::DecisionCreated { id, owner, source, .. } => {
                format!("{t} id={id} {} source={source:?}", owner.log())
            }
            Event::DecisionResolved { id, choices, .. } => {
                if let Some(c) = choices.first() {
                    format!("{t} id={id} chosen={c}")
                } else {
                    format!("{t} id={id}")
                }
            }

            Event::CrewCreated { id, agent, project, .. } => {
                format!("{t} id={id}{} agent={agent}", ns_fragment(project))
            }
            Event::CrewStarted { id, agent_id } => {
                format!("{t} id={id} agent_id={agent_id}")
            }
            Event::CrewUpdated { id, status, reason } => {
                if let Some(reason) = reason {
                    format!("{t} id={id} status={status} reason={reason}")
                } else {
                    format!("{t} id={id} status={status}")
                }
            }
            Event::CrewResume { id, message, kill } => {
                if *kill {
                    format!("{t} id={id} kill=true")
                } else if message.is_some() {
                    format!("{t} id={id} msg=true")
                } else {
                    format!("{t} id={id}")
                }
            }
            Event::CrewDeleted { id } => format!("{t} id={id}"),
        }
    }

    pub fn job_id(&self) -> Option<&JobId> {
        match self {
            Event::CommandRun { owner, .. } => owner.as_job(),
            Event::ShellExited { job_id, .. } => Some(job_id),

            Event::JobCreated { id, .. }
            | Event::JobAdvanced { id, .. }
            | Event::JobUpdated { id, .. }
            | Event::JobResume { id, .. }
            | Event::JobFailing { id, .. }
            | Event::JobCancelling { id, .. }
            | Event::JobCancel { id, .. }
            | Event::JobSuspending { id, .. }
            | Event::JobSuspend { id, .. }
            | Event::JobDeleted { id, .. } => Some(id),

            Event::StepStarted { job_id, .. }
            | Event::StepWaiting { job_id, .. }
            | Event::StepCompleted { job_id, .. }
            | Event::StepFailed { job_id, .. } => Some(job_id),

            Event::CronOnce { owner, .. }
            | Event::CronFired { owner, .. }
            | Event::DecisionCreated { owner, .. }
            | Event::WorkerDispatched { owner, .. }
            | Event::AgentSpawned { owner, .. }
            | Event::AgentSpawnFailed { owner, .. } => owner.as_job(),

            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "method_tests.rs"]
mod method_tests;

#[cfg(test)]
#[path = "logging_tests.rs"]
mod logging_tests;
