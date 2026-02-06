// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Step event helpers

use super::Event;
use crate::job::JobId;

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        Event::StepStarted { job_id, step, .. } => format!("{t} job={job_id} step={step}"),
        Event::StepWaiting { job_id, step, .. } => format!("{t} job={job_id} step={step}"),
        Event::StepCompleted { job_id, step } => format!("{t} job={job_id} step={step}"),
        Event::StepFailed { job_id, step, .. } => format!("{t} job={job_id} step={step}"),
        _ => unreachable!("not a step event"),
    }
}

pub(super) fn job_id(event: &Event) -> Option<&JobId> {
    match event {
        Event::StepStarted { job_id, .. }
        | Event::StepWaiting { job_id, .. }
        | Event::StepCompleted { job_id, .. }
        | Event::StepFailed { job_id, .. } => Some(job_id),
        _ => unreachable!("not a step event"),
    }
}
