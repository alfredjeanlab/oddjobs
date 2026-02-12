// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent lifecycle event handlers.

use oj_core::{AgentRecordStatus, Event, OwnerId, StepStatus};

use super::helpers;
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::AgentWorking { id: agent_id, owner, .. } => {
            // Route by owner; standalone agent status is
            // handled via CrewUpdated events.
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

        Event::AgentWaiting { id: agent_id, .. } => {
            if let Some(rec) = state.agents.get_mut(agent_id.as_str()) {
                rec.status = AgentRecordStatus::Idle;
                rec.updated_at_ms = helpers::epoch_ms_now();
            }
        }

        Event::AgentExited { id: agent_id, exit_code, owner, .. } => {
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

        Event::AgentFailed { id: agent_id, error, owner, .. } => {
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

        Event::AgentGone { id: agent_id, owner, .. } => {
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

        _ => {}
    }
}
