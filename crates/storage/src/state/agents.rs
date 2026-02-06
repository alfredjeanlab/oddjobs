// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent lifecycle event handlers.

use oj_core::{job::AgentSignal, AgentRecordStatus, AgentSignalKind, Event, OwnerId, StepStatus};

use super::helpers;
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::AgentWorking {
            agent_id, owner, ..
        } => {
            // Route by owner; standalone agent status is
            // handled via AgentRunStatusChanged events.
            if let OwnerId::Job(job_id) = owner {
                if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                    job.step_status = StepStatus::Running;
                }
            }
            // Update unified agent record
            if let Some(rec) = state.agents.get_mut(agent_id.as_str()) {
                rec.status = AgentRecordStatus::Running;
                rec.updated_at_ms = helpers::epoch_ms_now();
            }
        }

        Event::AgentWaiting { agent_id, .. } => {
            if let Some(rec) = state.agents.get_mut(agent_id.as_str()) {
                rec.status = AgentRecordStatus::Idle;
                rec.updated_at_ms = helpers::epoch_ms_now();
            }
        }

        Event::AgentExited {
            agent_id,
            exit_code,
            owner,
            ..
        } => {
            if let OwnerId::Job(job_id) = owner {
                helpers::apply_if_not_terminal(&mut state.jobs, job_id.as_str(), |job| {
                    if *exit_code == Some(0) {
                        job.step_status = StepStatus::Completed;
                    } else {
                        job.step_status = StepStatus::Failed;
                        job.error = Some(format!("exit code: {:?}", exit_code));
                    }
                });
            }
            if let Some(rec) = state.agents.get_mut(agent_id.as_str()) {
                rec.status = AgentRecordStatus::Exited;
                rec.updated_at_ms = helpers::epoch_ms_now();
            }
        }

        Event::AgentFailed {
            agent_id,
            error,
            owner,
            ..
        } => {
            if let OwnerId::Job(job_id) = owner {
                helpers::apply_if_not_terminal(&mut state.jobs, job_id.as_str(), |job| {
                    job.step_status = StepStatus::Failed;
                    job.error = Some(error.to_string());
                });
            }
            if let Some(rec) = state.agents.get_mut(agent_id.as_str()) {
                rec.status = AgentRecordStatus::Exited;
                rec.updated_at_ms = helpers::epoch_ms_now();
            }
        }

        Event::AgentGone {
            agent_id, owner, ..
        } => {
            if let OwnerId::Job(job_id) = owner {
                helpers::apply_if_not_terminal(&mut state.jobs, job_id.as_str(), |job| {
                    job.step_status = StepStatus::Failed;
                    job.error = Some("session terminated unexpectedly".to_string());
                });
            }
            if let Some(rec) = state.agents.get_mut(agent_id.as_str()) {
                rec.status = AgentRecordStatus::Gone;
                rec.updated_at_ms = helpers::epoch_ms_now();
            }
        }

        Event::AgentSignal {
            agent_id,
            kind,
            message,
        } => {
            // Continue is a no-op acknowledgement — don't store it so that
            // query_agent_signal still returns signaled=false (keeping the
            // stop hook blocking and the agent alive).
            if *kind == AgentSignalKind::Continue {
                return;
            }

            // Check standalone agent runs first
            let found_agent_run = state
                .agent_runs
                .values_mut()
                .find(|r| r.agent_id.as_deref() == Some(agent_id.as_str()));
            if let Some(run) = found_agent_run {
                run.action_tracker.agent_signal = Some(AgentSignal {
                    kind: kind.clone(),
                    message: message.clone(),
                });
            } else {
                // Find job by agent_id in current step
                let found_job = state.jobs.values_mut().find(|p| {
                    p.step_history.last().and_then(|r| r.agent_id.as_deref())
                        == Some(agent_id.as_str())
                });
                if let Some(job) = found_job {
                    job.action_tracker.agent_signal = Some(AgentSignal {
                        kind: kind.clone(),
                        message: message.clone(),
                    });
                }
            }
        }

        _ => {}
    }
}
