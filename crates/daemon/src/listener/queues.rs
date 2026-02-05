// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue request handlers.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use oj_core::{scoped_name, Event};
use oj_runbook::QueueType;
use oj_storage::{MaterializedState, QueueItemStatus};

use crate::event_bus::EventBus;
use crate::protocol::{QueueItemEntry, Response};

use super::suggest;
use super::workers::hash_runbook;
use super::ConnectionError;

/// Handle a QueuePush request.
pub(super) fn handle_queue_push(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    data: serde_json::Value,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_root, queue_name, namespace, "oj queue push", state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Validate queue exists
    let queue_def = match runbook.get_queue(queue_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown queue: {}", queue_name),
            })
        }
    };

    // External queues: wake workers to re-run the list command (no data needed)
    if queue_def.queue_type != QueueType::Persisted {
        wake_attached_workers(
            project_root,
            namespace,
            queue_name,
            &runbook,
            event_bus,
            state,
        )?;

        return Ok(Response::Ok);
    }

    // Validate data is a JSON object
    let obj = match data.as_object() {
        Some(o) => o,
        None => {
            return Ok(Response::Error {
                message: "data must be a JSON object".to_string(),
            })
        }
    };

    // Check required vars are present
    let data_keys: std::collections::HashSet<&str> = obj.keys().map(|k| k.as_str()).collect();
    let missing: Vec<&str> = queue_def
        .vars
        .iter()
        .filter(|v| !data_keys.contains(v.as_str()) && !queue_def.defaults.contains_key(v.as_str()))
        .map(|v| v.as_str())
        .collect();
    if !missing.is_empty() {
        return Ok(Response::Error {
            message: format!("missing required fields: {}", missing.join(", ")),
        });
    }

    // Build HashMap<String, String> from data, applying defaults for missing optional fields
    let mut final_data: std::collections::HashMap<String, String> = obj
        .iter()
        .map(|(k, v)| {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (k.clone(), s)
        })
        .collect();
    for (key, default_val) in &queue_def.defaults {
        if !final_data.contains_key(key) {
            final_data.insert(key.clone(), default_val.clone());
        }
    }

    // Deduplicate: if a pending or active item with the same data already exists, return it
    {
        let st = state.lock();
        let key = scoped_name(namespace, queue_name);
        if let Some(items) = st.queue_items.get(&key) {
            if let Some(existing) = items.iter().find(|i| {
                (i.status == oj_storage::QueueItemStatus::Pending
                    || i.status == oj_storage::QueueItemStatus::Active)
                    && i.data == final_data
            }) {
                let existing_id = existing.id.clone();
                drop(st);

                // Still wake workers so they can pick up pending work
                wake_attached_workers(
                    project_root,
                    namespace,
                    queue_name,
                    &runbook,
                    event_bus,
                    state,
                )?;

                return Ok(Response::QueuePushed {
                    queue_name: queue_name.to_string(),
                    item_id: existing_id,
                });
            }
        }
    }

    // Generate item ID
    let item_id = uuid::Uuid::new_v4().to_string();

    // Get current epoch ms
    let pushed_at_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Emit QueuePushed event
    let event = Event::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id: item_id.clone(),
        data: final_data,
        pushed_at_epoch_ms,
        namespace: namespace.to_string(),
    };
    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    // Wake workers attached to this queue (auto-starting stopped workers)
    wake_attached_workers(
        project_root,
        namespace,
        queue_name,
        &runbook,
        event_bus,
        state,
    )?;

    Ok(Response::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id,
    })
}

/// Wake or auto-start workers that are attached to the given queue.
///
/// For workers already running, emits `WorkerWake`. For workers that are
/// stopped or never started, emits `RunbookLoaded` + `WorkerStarted` (the
/// same events `handle_worker_start()` produces), effectively auto-starting
/// the worker on queue push.
fn wake_attached_workers(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    runbook: &oj_runbook::Runbook,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<(), ConnectionError> {
    // Find workers in the runbook that source from this queue
    let worker_names: Vec<&str> = runbook
        .workers
        .iter()
        .filter(|(_, w)| w.source.queue == queue_name)
        .map(|(name, _)| name.as_str())
        .collect();

    for name in &worker_names {
        let scoped = scoped_name(namespace, name);
        let is_running = {
            let state = state.lock();
            state
                .workers
                .get(&scoped)
                .map(|r| r.status == "running")
                .unwrap_or(false)
        };

        if is_running {
            // Existing behavior: wake the running worker
            tracing::info!(
                queue = queue_name,
                worker = *name,
                "waking running worker on queue push"
            );
            let event = Event::WorkerWake {
                worker_name: (*name).to_string(),
                namespace: namespace.to_string(),
            };
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;
        } else {
            // Auto-start: emit RunbookLoaded + WorkerStarted (same as handle_worker_start)
            let Some(worker_def) = runbook.get_worker(name) else {
                continue;
            };
            let (runbook_json, runbook_hash) =
                hash_runbook(runbook).map_err(ConnectionError::Internal)?;

            event_bus
                .send(Event::RunbookLoaded {
                    hash: runbook_hash.clone(),
                    version: 1,
                    runbook: runbook_json,
                })
                .map_err(|_| ConnectionError::WalError)?;

            event_bus
                .send(Event::WorkerStarted {
                    worker_name: (*name).to_string(),
                    project_root: project_root.to_path_buf(),
                    runbook_hash,
                    queue_name: worker_def.source.queue.clone(),
                    concurrency: worker_def.concurrency,
                    namespace: namespace.to_string(),
                })
                .map_err(|_| ConnectionError::WalError)?;

            tracing::info!(
                queue = queue_name,
                worker = *name,
                "auto-started worker on queue push"
            );
        }
    }

    if worker_names.is_empty() {
        tracing::warn!(
            queue = queue_name,
            "wake_attached_workers: no workers in runbook for queue"
        );
    }

    Ok(())
}

/// Resolve a queue item ID by exact match or unique prefix.
///
/// Returns the full item ID on success, or an error response if the item
/// is not found or the prefix is ambiguous.
fn resolve_queue_item_id(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
) -> Result<String, Response> {
    let state = state.lock();
    let key = scoped_name(namespace, queue_name);
    let items = state.queue_items.get(&key);

    // Try exact match first
    if let Some(item) = items.and_then(|items| items.iter().find(|i| i.id == item_id)) {
        return Ok(item.id.clone());
    }

    // Try prefix match
    let matches: Vec<_> = items
        .map(|items| {
            items
                .iter()
                .filter(|i| i.id.starts_with(item_id))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    match matches.len() {
        1 => Ok(matches[0].id.clone()),
        0 => Err(Response::Error {
            message: format!("item '{}' not found in queue '{}'", item_id, queue_name),
        }),
        n => Err(Response::Error {
            message: format!(
                "ambiguous item ID '{}': {} matches in queue '{}'",
                item_id, n, queue_name
            ),
        }),
    }
}

/// Handle a QueueDrop request.
pub(super) fn handle_queue_drop(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (runbook, _effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_root, queue_name, namespace, "oj queue drop", state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };

    // Validate queue exists
    let queue_def = match runbook.get_queue(queue_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown queue: {}", queue_name),
            })
        }
    };

    // Validate queue is persisted
    if queue_def.queue_type != QueueType::Persisted {
        return Ok(Response::Error {
            message: format!("queue '{}' is not a persisted queue", queue_name),
        });
    }

    // Resolve item ID (exact or prefix match)
    let resolved_id = match resolve_queue_item_id(state, namespace, queue_name, item_id) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    // Emit QueueDropped event
    let event = Event::QueueDropped {
        queue_name: queue_name.to_string(),
        item_id: resolved_id.clone(),
        namespace: namespace.to_string(),
    };
    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::QueueDropped {
        queue_name: queue_name.to_string(),
        item_id: resolved_id,
    })
}

/// Handle a QueueRetry request (single or bulk).
// TODO(refactor): Consolidate filter args into a RetryOptions struct once patterns stabilize
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_queue_retry(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_ids: &[String],
    all_dead: bool,
    status_filter: Option<&str>,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_root, queue_name, namespace, "oj queue retry", state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Validate queue exists
    let queue_def = match runbook.get_queue(queue_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown queue: {}", queue_name),
            })
        }
    };

    // Validate queue is persisted
    if queue_def.queue_type != QueueType::Persisted {
        return Ok(Response::Error {
            message: format!("queue '{}' is not a persisted queue", queue_name),
        });
    }

    // Determine which items to retry
    let items_to_process: Vec<String> = if all_dead || status_filter.is_some() {
        // Filter mode: collect items by status
        let st = state.lock();
        let key = scoped_name(namespace, queue_name);
        st.queue_items
            .get(&key)
            .map(|items| {
                items
                    .iter()
                    .filter(|i| {
                        if all_dead {
                            i.status == QueueItemStatus::Dead
                        } else if let Some(filter) = status_filter {
                            match filter.to_lowercase().as_str() {
                                "dead" => i.status == QueueItemStatus::Dead,
                                "failed" => i.status == QueueItemStatus::Failed,
                                _ => false,
                            }
                        } else {
                            false
                        }
                    })
                    .map(|i| i.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        // Explicit IDs mode
        item_ids.to_vec()
    };

    // If no items to process and using filters, return early with empty result
    if items_to_process.is_empty() && (all_dead || status_filter.is_some()) {
        return Ok(Response::QueueItemsRetried {
            queue_name: queue_name.to_string(),
            item_ids: vec![],
            already_retried: vec![],
            not_found: vec![],
        });
    }

    // For backward compatibility: single item without filters uses old response
    if items_to_process.len() == 1 && !all_dead && status_filter.is_none() {
        let item_id = &items_to_process[0];

        // Resolve item ID (exact or prefix match)
        let resolved_id = match resolve_queue_item_id(state, namespace, queue_name, item_id) {
            Ok(id) => id,
            Err(resp) => return Ok(resp),
        };

        // Validate item is in Dead or Failed status
        {
            let st = state.lock();
            let key = scoped_name(namespace, queue_name);
            let item = st
                .queue_items
                .get(&key)
                .and_then(|items| items.iter().find(|i| i.id == resolved_id));
            if let Some(item) = item {
                if item.status != QueueItemStatus::Dead && item.status != QueueItemStatus::Failed {
                    return Ok(Response::Error {
                        message: format!(
                            "item '{}' is {:?}, only dead or failed items can be retried",
                            resolved_id, item.status
                        ),
                    });
                }
            }
        }

        // Emit QueueItemRetry event
        let event = Event::QueueItemRetry {
            queue_name: queue_name.to_string(),
            item_id: resolved_id.clone(),
            namespace: namespace.to_string(),
        };
        event_bus
            .send(event)
            .map_err(|_| ConnectionError::WalError)?;

        // Wake workers attached to this queue
        wake_attached_workers(
            project_root,
            namespace,
            queue_name,
            &runbook,
            event_bus,
            state,
        )?;

        return Ok(Response::QueueRetried {
            queue_name: queue_name.to_string(),
            item_id: resolved_id,
        });
    }

    // Bulk retry: process multiple items
    let mut retried = Vec::new();
    let mut already_retried = Vec::new();
    let mut not_found = Vec::new();

    for item_id in &items_to_process {
        // Resolve item ID (exact or prefix match)
        let resolved_id = match resolve_queue_item_id(state, namespace, queue_name, item_id) {
            Ok(id) => id,
            Err(_) => {
                not_found.push(item_id.clone());
                continue;
            }
        };

        // Check item status
        let item_status = {
            let st = state.lock();
            let key = scoped_name(namespace, queue_name);
            st.queue_items
                .get(&key)
                .and_then(|items| items.iter().find(|i| i.id == resolved_id))
                .map(|i| i.status.clone())
        };

        match item_status {
            Some(QueueItemStatus::Dead) | Some(QueueItemStatus::Failed) => {
                // Emit QueueItemRetry event
                let event = Event::QueueItemRetry {
                    queue_name: queue_name.to_string(),
                    item_id: resolved_id.clone(),
                    namespace: namespace.to_string(),
                };
                event_bus
                    .send(event)
                    .map_err(|_| ConnectionError::WalError)?;
                retried.push(resolved_id);
            }
            Some(_) => {
                already_retried.push(resolved_id);
            }
            None => {
                not_found.push(item_id.clone());
            }
        }
    }

    // Wake workers if any items were retried
    if !retried.is_empty() {
        wake_attached_workers(
            project_root,
            namespace,
            queue_name,
            &runbook,
            event_bus,
            state,
        )?;
    }

    Ok(Response::QueueItemsRetried {
        queue_name: queue_name.to_string(),
        item_ids: retried,
        already_retried,
        not_found,
    })
}

/// Handle a QueueRetryBulk request.
///
/// Retries multiple dead or failed queue items. Supports:
/// - Specific item IDs (with prefix resolution)
/// - `all_dead` flag to retry all dead items
/// - `status_filter` to retry items by status (e.g., "dead", "failed")
// TODO(refactor): consider grouping filter options (item_ids, all_dead, status_filter) into a struct
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_queue_retry_bulk(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_ids: &[String],
    all_dead: bool,
    status_filter: Option<&str>,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_root, queue_name, namespace, "oj queue retry", state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Validate queue exists
    let queue_def = match runbook.get_queue(queue_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown queue: {}", queue_name),
            })
        }
    };

    // Validate queue is persisted
    if queue_def.queue_type != QueueType::Persisted {
        return Ok(Response::Error {
            message: format!("queue '{}' is not a persisted queue", queue_name),
        });
    }

    // Collect items to retry based on mode
    let items_to_retry: Vec<(String, Option<QueueItemStatus>)> = {
        let st = state.lock();
        let key = scoped_name(namespace, queue_name);
        let items = st.queue_items.get(&key);

        if all_dead {
            // Retry all dead items
            items
                .map(|items| {
                    items
                        .iter()
                        .filter(|i| i.status == QueueItemStatus::Dead)
                        .map(|i| (i.id.clone(), Some(i.status.clone())))
                        .collect()
                })
                .unwrap_or_default()
        } else if let Some(status) = status_filter {
            // Filter by status
            let target_status = match status.to_lowercase().as_str() {
                "dead" => Some(QueueItemStatus::Dead),
                "failed" => Some(QueueItemStatus::Failed),
                _ => None,
            };
            if let Some(target) = target_status {
                items
                    .map(|items| {
                        items
                            .iter()
                            .filter(|i| i.status == target)
                            .map(|i| (i.id.clone(), Some(i.status.clone())))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                return Ok(Response::Error {
                    message: format!(
                        "invalid status filter '{}': must be 'dead' or 'failed'",
                        status
                    ),
                });
            }
        } else {
            // Resolve specific item IDs
            let mut resolved = Vec::new();
            for id in item_ids {
                // Try exact match first
                if let Some(item) = items.and_then(|items| items.iter().find(|i| i.id == *id)) {
                    resolved.push((item.id.clone(), Some(item.status.clone())));
                    continue;
                }

                // Try prefix match
                let matches: Vec<_> = items
                    .map(|items| {
                        items
                            .iter()
                            .filter(|i| i.id.starts_with(id))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                match matches.len() {
                    1 => resolved.push((matches[0].id.clone(), Some(matches[0].status.clone()))),
                    0 => resolved.push((id.clone(), None)), // Not found
                    _ => resolved.push((id.clone(), None)), // Ambiguous - will be skipped
                }
            }
            resolved
        }
    };

    // Process each item
    let mut item_ids = Vec::new();
    let mut already_retried = Vec::new();
    let mut not_found = Vec::new();

    for (item_id, status) in items_to_retry {
        match status {
            None => {
                // Item not found or ambiguous
                not_found.push(item_id);
            }
            Some(QueueItemStatus::Dead) | Some(QueueItemStatus::Failed) => {
                // Valid for retry - emit event
                let event = Event::QueueItemRetry {
                    queue_name: queue_name.to_string(),
                    item_id: item_id.clone(),
                    namespace: namespace.to_string(),
                };
                event_bus
                    .send(event)
                    .map_err(|_| ConnectionError::WalError)?;
                item_ids.push(item_id);
            }
            Some(_status) => {
                // Not in a retryable status
                already_retried.push(item_id);
            }
        }
    }

    // Wake workers attached to this queue (if any items were retried)
    if !item_ids.is_empty() {
        wake_attached_workers(
            project_root,
            namespace,
            queue_name,
            &runbook,
            event_bus,
            state,
        )?;
    }

    Ok(Response::QueueItemsRetried {
        queue_name: queue_name.to_string(),
        item_ids,
        already_retried,
        not_found,
    })
}

/// Handle a QueueDrain request.
///
/// Removes all pending items from a persisted queue and returns them.
pub(super) fn handle_queue_drain(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (runbook, _effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_root, queue_name, namespace, "oj queue drain", state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };

    // Validate queue exists
    let queue_def = match runbook.get_queue(queue_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown queue: {}", queue_name),
            })
        }
    };

    // Validate queue is persisted
    if queue_def.queue_type != QueueType::Persisted {
        return Ok(Response::Error {
            message: format!("queue '{}' is not a persisted queue", queue_name),
        });
    }

    // Collect pending item IDs and build response summaries
    let pending_items: Vec<crate::protocol::QueueItemSummary> = {
        let state = state.lock();
        let key = scoped_name(namespace, queue_name);
        state
            .queue_items
            .get(&key)
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i.status == oj_storage::QueueItemStatus::Pending)
                    .map(|i| crate::protocol::QueueItemSummary {
                        id: i.id.clone(),
                        status: "pending".to_string(),
                        data: i.data.clone(),
                        worker_name: i.worker_name.clone(),
                        pushed_at_epoch_ms: i.pushed_at_epoch_ms,
                        failure_count: i.failure_count,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    // Emit QueueDropped for each pending item
    for item in &pending_items {
        let event = Event::QueueDropped {
            queue_name: queue_name.to_string(),
            item_id: item.id.clone(),
            namespace: namespace.to_string(),
        };
        event_bus
            .send(event)
            .map_err(|_| ConnectionError::WalError)?;
    }

    Ok(Response::QueueDrained {
        queue_name: queue_name.to_string(),
        items: pending_items,
    })
}

/// Handle a QueueFail request — force-fail an active item.
pub(super) fn handle_queue_fail(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (_runbook, _effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_root, queue_name, namespace, "oj queue fail", state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };

    // Validate queue exists and is persisted
    let queue_def = match _runbook.get_queue(queue_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown queue: {}", queue_name),
            })
        }
    };
    if queue_def.queue_type != QueueType::Persisted {
        return Ok(Response::Error {
            message: format!("queue '{}' is not a persisted queue", queue_name),
        });
    }

    // Resolve item ID (exact or prefix match)
    let resolved_id = match resolve_queue_item_id(state, namespace, queue_name, item_id) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    // Validate item is Active
    {
        let st = state.lock();
        let key = scoped_name(namespace, queue_name);
        let item = st
            .queue_items
            .get(&key)
            .and_then(|items| items.iter().find(|i| i.id == resolved_id));
        if let Some(item) = item {
            use oj_storage::QueueItemStatus;
            if item.status != QueueItemStatus::Active {
                return Ok(Response::Error {
                    message: format!(
                        "item '{}' is {:?}, only active items can be force-failed",
                        resolved_id, item.status
                    ),
                });
            }
        }
    }

    // Emit QueueFailed event
    let event = Event::QueueFailed {
        queue_name: queue_name.to_string(),
        item_id: resolved_id.clone(),
        error: "force-failed via oj queue fail".to_string(),
        namespace: namespace.to_string(),
    };
    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::QueueFailed {
        queue_name: queue_name.to_string(),
        item_id: resolved_id,
    })
}

/// Handle a QueueDone request — force-complete an active item.
pub(super) fn handle_queue_done(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (_runbook, _effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_root, queue_name, namespace, "oj queue done", state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };

    // Validate queue exists and is persisted
    let queue_def = match _runbook.get_queue(queue_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown queue: {}", queue_name),
            })
        }
    };
    if queue_def.queue_type != QueueType::Persisted {
        return Ok(Response::Error {
            message: format!("queue '{}' is not a persisted queue", queue_name),
        });
    }

    // Resolve item ID (exact or prefix match)
    let resolved_id = match resolve_queue_item_id(state, namespace, queue_name, item_id) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    // Validate item is Active
    {
        let st = state.lock();
        let key = scoped_name(namespace, queue_name);
        let item = st
            .queue_items
            .get(&key)
            .and_then(|items| items.iter().find(|i| i.id == resolved_id));
        if let Some(item) = item {
            use oj_storage::QueueItemStatus;
            if item.status != QueueItemStatus::Active {
                return Ok(Response::Error {
                    message: format!(
                        "item '{}' is {:?}, only active items can be force-completed",
                        resolved_id, item.status
                    ),
                });
            }
        }
    }

    // Emit QueueCompleted event
    let event = Event::QueueCompleted {
        queue_name: queue_name.to_string(),
        item_id: resolved_id.clone(),
        namespace: namespace.to_string(),
    };
    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::QueueCompleted {
        queue_name: queue_name.to_string(),
        item_id: resolved_id,
    })
}

/// Handle a QueuePrune request.
///
/// Removes completed and dead items from a persisted queue. By default, only
/// items older than 12 hours are pruned. The `all` flag removes all terminal
/// items regardless of age.
pub(super) fn handle_queue_prune(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    all: bool,
    dry_run: bool,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (runbook, _effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_root, queue_name, namespace, "oj queue prune", state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };

    // Validate queue exists
    let queue_def = match runbook.get_queue(queue_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown queue: {}", queue_name),
            })
        }
    };

    // Validate queue is persisted
    if queue_def.queue_type != QueueType::Persisted {
        return Ok(Response::Error {
            message: format!("queue '{}' is not a persisted queue", queue_name),
        });
    }

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_threshold_ms: u64 = 12 * 60 * 60 * 1000; // 12 hours

    // Collect terminal items (Completed, Dead)
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;
    {
        let state = state.lock();
        let key = scoped_name(namespace, queue_name);
        if let Some(items) = state.queue_items.get(&key) {
            for item in items {
                let is_terminal = matches!(
                    item.status,
                    QueueItemStatus::Completed | QueueItemStatus::Dead
                );
                if !is_terminal {
                    skipped += 1;
                    continue;
                }
                if !all && now_ms.saturating_sub(item.pushed_at_epoch_ms) < age_threshold_ms {
                    skipped += 1;
                    continue;
                }
                to_prune.push(QueueItemEntry {
                    queue_name: queue_name.to_string(),
                    item_id: item.id.clone(),
                    status: format!("{:?}", item.status).to_lowercase(),
                });
            }
        }
    }

    // Emit QueueDropped events (unless dry-run)
    if !dry_run {
        for entry in &to_prune {
            let event = Event::QueueDropped {
                queue_name: queue_name.to_string(),
                item_id: entry.item_id.clone(),
                namespace: namespace.to_string(),
            };
            event_bus
                .send(event)
                .map_err(|_| ConnectionError::WalError)?;
        }
    }

    Ok(Response::QueuesPruned {
        pruned: to_prune,
        skipped,
    })
}

#[cfg(test)]
#[path = "queues_tests/mod.rs"]
mod tests;

/// Load a runbook that contains the given queue name.
fn load_runbook_for_queue(
    project_root: &Path,
    queue_name: &str,
) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_root.join(".oj/runbooks");
    oj_runbook::find_runbook_by_queue(&runbook_dir, queue_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no runbook found containing queue '{}'", queue_name))
}

/// Generate a "did you mean" suggestion for a queue name.
fn suggest_for_queue(
    project_root: &Path,
    queue_name: &str,
    namespace: &str,
    command_prefix: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> String {
    let ns = namespace.to_string();
    let runbook_dir = project_root.join(".oj/runbooks");
    suggest::suggest_for_resource(
        queue_name,
        namespace,
        command_prefix,
        state,
        suggest::ResourceType::Queue,
        || {
            oj_runbook::collect_all_queues(&runbook_dir)
                .unwrap_or_default()
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        },
        |state| {
            state
                .queue_items
                .keys()
                .filter_map(|k| {
                    let (item_ns, name) = oj_core::split_scoped_name(k);
                    if item_ns == ns {
                        Some(name.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        },
    )
}
