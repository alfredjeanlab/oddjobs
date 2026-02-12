// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue event handlers.

use oj_core::{scoped_name, Event};

use super::helpers;
use super::types::{QueueItem, QueueItemStatus};
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::QueuePushed { queue, item_id, data, pushed_at_ms, project } => {
            let key = scoped_name(project, queue);
            let items = state.queue_items.entry(key).or_default();
            // Idempotency: skip if item already exists
            if !items.iter().any(|i| i.id == *item_id) {
                items.push(QueueItem {
                    id: item_id.clone(),
                    queue: queue.clone(),
                    data: data.clone(),
                    status: QueueItemStatus::Pending,
                    worker: None,
                    pushed_at_ms: *pushed_at_ms,
                    failures: 0,
                });
            }
        }

        Event::QueueTaken { queue, item_id, worker, project } => {
            let key = scoped_name(project, queue);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    item.status = QueueItemStatus::Active;
                    item.worker = Some(worker.clone());
                }
            }
        }

        Event::QueueCompleted { queue, item_id, project } => {
            let key = scoped_name(project, queue);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    item.status = QueueItemStatus::Completed;
                }
            }
        }

        Event::QueueFailed { queue, item_id, project, .. } => {
            let key = scoped_name(project, queue);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    // Idempotency: only increment failures on state transition
                    // (prevents double-increment when event is applied twice)
                    if item.status != QueueItemStatus::Failed {
                        item.failures += 1;
                    }
                    item.status = QueueItemStatus::Failed;
                }
            }
        }

        Event::QueueDropped { queue, item_id, project } => {
            let key = scoped_name(project, queue);
            if let Some(items) = state.queue_items.get_mut(&key) {
                items.retain(|i| i.id != *item_id);
            }
        }

        Event::QueueRetry { queue, item_id, project } => {
            let key = scoped_name(project, queue);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    item.status = QueueItemStatus::Pending;
                    item.failures = 0;
                    item.worker = None;
                }
            }
        }

        Event::QueueDead { queue, item_id, project } => {
            let key = scoped_name(project, queue);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    item.status = QueueItemStatus::Dead;
                }
            }
        }

        _ => {}
    }
}
