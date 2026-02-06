// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker and queue event helpers

use super::{ns_fragment, Event};
use crate::job::JobId;

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        // Worker events
        Event::WorkerStarted { worker_name, .. } => format!("{t} worker={worker_name}"),
        Event::WorkerWake { worker_name, .. } => format!("{t} worker={worker_name}"),
        Event::WorkerPollComplete {
            worker_name, items, ..
        } => format!("{t} worker={worker_name} items={}", items.len()),
        Event::WorkerTakeComplete {
            worker_name,
            item_id,
            exit_code,
            ..
        } => format!("{t} worker={worker_name} item={item_id} exit={exit_code}"),
        Event::WorkerItemDispatched {
            worker_name,
            item_id,
            job_id,
            ..
        } => format!("{t} worker={worker_name} item={item_id} job={job_id}"),
        Event::WorkerStopped { worker_name, .. } => format!("{t} worker={worker_name}"),
        Event::WorkerResized {
            worker_name,
            concurrency,
            namespace,
        } => {
            format!(
                "{t} worker={worker_name}{} concurrency={concurrency}",
                ns_fragment(namespace)
            )
        }
        Event::WorkerDeleted {
            worker_name,
            namespace,
        } => {
            format!("{t} worker={worker_name}{}", ns_fragment(namespace))
        }
        // Queue events
        Event::QueuePushed {
            queue_name,
            item_id,
            ..
        }
        | Event::QueueTaken {
            queue_name,
            item_id,
            ..
        }
        | Event::QueueCompleted {
            queue_name,
            item_id,
            ..
        }
        | Event::QueueFailed {
            queue_name,
            item_id,
            ..
        }
        | Event::QueueDropped {
            queue_name,
            item_id,
            ..
        }
        | Event::QueueItemRetry {
            queue_name,
            item_id,
            ..
        }
        | Event::QueueItemDead {
            queue_name,
            item_id,
            ..
        } => format!("{t} queue={queue_name} item={item_id}"),
        _ => unreachable!("not a worker/queue event"),
    }
}

pub(super) fn job_id(event: &Event) -> Option<&JobId> {
    match event {
        Event::WorkerItemDispatched { job_id, .. } => Some(job_id),
        _ => unreachable!("not a worker dispatch event"),
    }
}
