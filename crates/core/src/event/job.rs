// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job event helpers

use super::{ns_fragment, Event};
use crate::job::JobId;

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        Event::JobCreated {
            id,
            kind,
            name,
            namespace,
            ..
        } => {
            format!(
                "{t} id={id}{} kind={kind} name={name}",
                ns_fragment(namespace)
            )
        }
        Event::JobAdvanced { id, step } => format!("{t} id={id} step={step}"),
        Event::JobUpdated { id, .. } => format!("{t} id={id}"),
        Event::JobResume { id, .. } => format!("{t} id={id}"),
        Event::JobCancelling { id } => format!("{t} id={id}"),
        Event::JobCancel { id } => format!("{t} id={id}"),
        Event::JobSuspending { id } => format!("{t} id={id}"),
        Event::JobSuspend { id } => format!("{t} id={id}"),
        Event::JobDeleted { id } => format!("{t} id={id}"),
        _ => unreachable!("not a job event"),
    }
}

pub(super) fn job_id(event: &Event) -> Option<&JobId> {
    match event {
        Event::JobCreated { id, .. }
        | Event::JobAdvanced { id, .. }
        | Event::JobUpdated { id, .. }
        | Event::JobResume { id, .. }
        | Event::JobCancelling { id, .. }
        | Event::JobCancel { id, .. }
        | Event::JobSuspending { id, .. }
        | Event::JobSuspend { id, .. }
        | Event::JobDeleted { id, .. } => Some(id),
        _ => unreachable!("not a job event"),
    }
}
