// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue event handlers.

use oj_core::{scoped_name, Event};

use super::helpers;
use super::types::{QueueItem, QueueItemStatus};
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::QueuePushed {
            queue_name,
            item_id,
            data,
            pushed_at_epoch_ms,
            namespace,
        } => {
            let key = scoped_name(namespace, queue_name);
            let items = state.queue_items.entry(key).or_default();
            // Idempotency: skip if item already exists
            if !items.iter().any(|i| i.id == *item_id) {
                items.push(QueueItem {
                    id: item_id.clone(),
                    queue_name: queue_name.clone(),
                    data: data.clone(),
                    status: QueueItemStatus::Pending,
                    worker_name: None,
                    pushed_at_epoch_ms: *pushed_at_epoch_ms,
                    failure_count: 0,
                });
            }
        }

        Event::QueueTaken {
            queue_name,
            item_id,
            worker_name,
            namespace,
        } => {
            let key = scoped_name(namespace, queue_name);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    item.status = QueueItemStatus::Active;
                    item.worker_name = Some(worker_name.clone());
                }
            }
        }

        Event::QueueCompleted {
            queue_name,
            item_id,
            namespace,
        } => {
            let key = scoped_name(namespace, queue_name);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    item.status = QueueItemStatus::Completed;
                }
            }
        }

        Event::QueueFailed {
            queue_name,
            item_id,
            namespace,
            ..
        } => {
            let key = scoped_name(namespace, queue_name);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    // Idempotency: only increment failure_count on state transition
                    // (prevents double-increment when event is applied twice)
                    if item.status != QueueItemStatus::Failed {
                        item.failure_count += 1;
                    }
                    item.status = QueueItemStatus::Failed;
                }
            }
        }

        Event::QueueDropped {
            queue_name,
            item_id,
            namespace,
        } => {
            let key = scoped_name(namespace, queue_name);
            if let Some(items) = state.queue_items.get_mut(&key) {
                items.retain(|i| i.id != *item_id);
            }
        }

        Event::QueueItemRetry {
            queue_name,
            item_id,
            namespace,
        } => {
            let key = scoped_name(namespace, queue_name);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    item.status = QueueItemStatus::Pending;
                    item.failure_count = 0;
                    item.worker_name = None;
                }
            }
        }

        Event::QueueItemDead {
            queue_name,
            item_id,
            namespace,
        } => {
            let key = scoped_name(namespace, queue_name);
            if let Some(items) = state.queue_items.get_mut(&key) {
                if let Some(item) = helpers::find_queue_item_mut(items, item_id) {
                    item.status = QueueItemStatus::Dead;
                }
            }
        }

        _ => {}
    }
}
