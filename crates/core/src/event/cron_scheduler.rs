// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron event helpers

use super::{ns_fragment, Event};
use crate::job::JobId;

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        Event::CronStarted { cron_name, .. } => format!("{t} cron={cron_name}"),
        Event::CronStopped { cron_name, .. } => format!("{t} cron={cron_name}"),
        Event::CronOnce {
            cron_name,
            job_id,
            agent_name,
            ..
        } => {
            if let Some(agent) = agent_name {
                format!("{t} cron={cron_name} agent={agent}")
            } else {
                format!("{t} cron={cron_name} job={job_id}")
            }
        }
        Event::CronFired {
            cron_name,
            job_id,
            agent_run_id,
            ..
        } => {
            if let Some(ar_id) = agent_run_id {
                format!("{t} cron={cron_name} agent_run={ar_id}")
            } else {
                format!("{t} cron={cron_name} job={job_id}")
            }
        }
        Event::CronDeleted {
            cron_name,
            namespace,
        } => {
            format!("{t} cron={cron_name}{}", ns_fragment(namespace))
        }
        _ => unreachable!("not a cron event"),
    }
}

pub(super) fn job_id(event: &Event) -> Option<&JobId> {
    match event {
        Event::CronOnce {
            job_id, agent_name, ..
        } => {
            if agent_name.is_some() {
                None
            } else {
                Some(job_id)
            }
        }
        Event::CronFired {
            job_id,
            agent_run_id,
            ..
        } => {
            if agent_run_id.is_some() {
                None
            } else {
                Some(job_id)
            }
        }
        _ => unreachable!("not a cron event with job_id"),
    }
}
