// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision event helpers

use super::Event;
use crate::job::JobId;
use crate::owner::OwnerId;

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        Event::DecisionCreated {
            id,
            job_id,
            owner,
            source,
            ..
        } => match owner {
            OwnerId::AgentRun(ar_id) => {
                format!("{t} id={id} agent_run={ar_id} source={source:?}")
            }
            OwnerId::Job(_) => format!("{t} id={id} job={job_id} source={source:?}"),
        },
        Event::DecisionResolved { id, chosen, .. } => {
            if let Some(c) = chosen {
                format!("{t} id={id} chosen={c}")
            } else {
                format!("{t} id={id}")
            }
        }
        _ => unreachable!("not a decision event"),
    }
}

pub(super) fn job_id(event: &Event) -> Option<&JobId> {
    match event {
        Event::DecisionCreated { job_id, owner, .. } => {
            if matches!(owner, OwnerId::AgentRun(_)) {
                None
            } else {
                Some(job_id)
            }
        }
        _ => unreachable!("not a decision event with job_id"),
    }
}
