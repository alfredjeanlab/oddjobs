// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Core event helpers for session, command, runbook, shell, system, and timer events

use super::{ns_fragment, Event};
use crate::id::ShortId;
use crate::job::JobId;
use crate::owner::OwnerId;

pub(super) fn is_empty_map<K, V>(map: &std::collections::HashMap<K, V>) -> bool {
    map.is_empty()
}

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        Event::CommandRun {
            job_id,
            command,
            namespace,
            ..
        } => {
            format!("{t} id={job_id}{} cmd={command}", ns_fragment(namespace))
        }
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
        Event::SessionCreated { id, owner } => match owner {
            OwnerId::Job(job_id) => format!("{t} id={id} job={job_id}"),
            OwnerId::AgentRun(ar_id) => format!("{t} id={id} agent_run={ar_id}"),
        },
        Event::SessionInput { id, .. } => format!("{t} id={id}"),
        Event::SessionDeleted { id } => format!("{t} id={id}"),
        Event::ShellExited {
            job_id,
            step,
            exit_code,
            ..
        } => format!("{t} job={job_id} step={step} exit={exit_code}"),
        Event::Shutdown | Event::Custom => t.to_string(),
        Event::TimerStart { id } => format!("{t} id={id}"),
        _ => unreachable!("not a core event"),
    }
}

pub(super) fn job_id(event: &Event) -> Option<&JobId> {
    match event {
        Event::CommandRun { job_id, .. } | Event::ShellExited { job_id, .. } => Some(job_id),
        Event::SessionCreated { owner, .. } => match owner {
            OwnerId::Job(id) => Some(id),
            OwnerId::AgentRun(_) => None,
        },
        _ => unreachable!("not a core event with job_id"),
    }
}
