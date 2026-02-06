// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue request handlers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use oj_core::{scoped_name, Event};
use oj_runbook::QueueType;
use oj_storage::{MaterializedState, QueueItemStatus};

use crate::protocol::{QueueItemEntry, Response};

use super::mutations::emit;
use super::suggest;
use super::workers::hash_and_emit_runbook;
use super::ConnectionError;
use super::ListenCtx;

/// Handle a QueuePush request.
pub(super) fn handle_queue_push(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    data: serde_json::Value,
) -> Result<Response, ConnectionError> {
    // Load runbook containing the queue.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        &ctx.state,
        |root| load_runbook_for_queue(root, queue_name),
        || {
            suggest_for_queue(
                project_root,
                queue_name,
                namespace,
                "oj queue push",
                &ctx.state,
            )
        },
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
        wake_attached_workers(ctx, project_root, namespace, queue_name, &runbook)?;

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
        let st = ctx.state.lock();
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
                wake_attached_workers(ctx, project_root, namespace, queue_name, &runbook)?;

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
    emit(&ctx.event_bus, event)?;

    // Wake workers attached to this queue (auto-starting stopped workers)
    wake_attached_workers(ctx, project_root, namespace, queue_name, &runbook)?;

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
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    runbook: &oj_runbook::Runbook,
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
            let state = ctx.state.lock();
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
            emit(
                &ctx.event_bus,
                Event::WorkerWake {
                    worker_name: (*name).to_string(),
                    namespace: namespace.to_string(),
                },
            )?;
        } else {
            // Auto-start: emit RunbookLoaded + WorkerStarted (same as handle_worker_start)
            let Some(worker_def) = runbook.get_worker(name) else {
                continue;
            };
            let runbook_hash = hash_and_emit_runbook(&ctx.event_bus, runbook)?;

            emit(
                &ctx.event_bus,
                Event::WorkerStarted {
                    worker_name: (*name).to_string(),
                    project_root: project_root.to_path_buf(),
                    runbook_hash,
                    queue_name: worker_def.source.queue.clone(),
                    concurrency: worker_def.concurrency,
                    namespace: namespace.to_string(),
                },
            )?;

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
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
) -> Result<Response, ConnectionError> {
    // Load runbook (optional — queue may exist only in persisted state).
    let runbook_info = match load_runbook_for_queue_or_state(
        ctx,
        project_root,
        namespace,
        queue_name,
        "oj queue drop",
    ) {
        Ok(info) => info,
        Err(resp) => return Ok(resp),
    };

    // Validate queue is persisted (if runbook is available)
    if let Err(resp) = validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
    }

    // Resolve item ID (exact or prefix match)
    let resolved_id = match resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    // Emit QueueDropped event
    let event = Event::QueueDropped {
        queue_name: queue_name.to_string(),
        item_id: resolved_id.clone(),
        namespace: namespace.to_string(),
    };
    emit(&ctx.event_bus, event)?;

    Ok(Response::QueueDropped {
        queue_name: queue_name.to_string(),
        item_id: resolved_id,
    })
}

/// Filter parameters for queue retry operations.
pub(super) struct RetryFilter<'a> {
    pub item_ids: &'a [String],
    pub all_dead: bool,
    pub status_filter: Option<&'a str>,
}

/// Handle a QueueRetry request (single or bulk).
pub(super) fn handle_queue_retry(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    filter: RetryFilter<'_>,
) -> Result<Response, ConnectionError> {
    let RetryFilter {
        item_ids,
        all_dead,
        status_filter,
    } = filter;
    // Load runbook (optional — queue may exist only in persisted state).
    let runbook_info = match load_runbook_for_queue_or_state(
        ctx,
        project_root,
        namespace,
        queue_name,
        "oj queue retry",
    ) {
        Ok(info) => info,
        Err(resp) => return Ok(resp),
    };

    // Validate queue is persisted (if runbook is available)
    if let Err(resp) = validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
    }

    // Determine which items to retry
    let items_to_process: Vec<String> = if all_dead || status_filter.is_some() {
        // Filter mode: collect items by status
        let st = ctx.state.lock();
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
        let resolved_id = match resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
            Ok(id) => id,
            Err(resp) => return Ok(resp),
        };

        // Validate item is in Dead or Failed status
        {
            let st = ctx.state.lock();
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
        emit(&ctx.event_bus, event)?;

        // Wake workers attached to this queue (if runbook is available)
        if let Some((ref runbook, ref effective_root)) = runbook_info {
            wake_attached_workers(ctx, effective_root, namespace, queue_name, runbook)?;
        }

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
        let resolved_id = match resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
            Ok(id) => id,
            Err(_) => {
                not_found.push(item_id.clone());
                continue;
            }
        };

        // Check item status
        let item_status = {
            let st = ctx.state.lock();
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
                emit(&ctx.event_bus, event)?;
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

    // Wake workers if any items were retried (and runbook is available)
    if !retried.is_empty() {
        if let Some((ref runbook, ref effective_root)) = runbook_info {
            wake_attached_workers(ctx, effective_root, namespace, queue_name, runbook)?;
        }
    }

    Ok(Response::QueueItemsRetried {
        queue_name: queue_name.to_string(),
        item_ids: retried,
        already_retried,
        not_found,
    })
}

/// Handle a QueueDrain request.
///
/// Removes all pending items from a persisted queue and returns them.
pub(super) fn handle_queue_drain(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
) -> Result<Response, ConnectionError> {
    // Load runbook (optional — queue may exist only in persisted state).
    let runbook_info = match load_runbook_for_queue_or_state(
        ctx,
        project_root,
        namespace,
        queue_name,
        "oj queue drain",
    ) {
        Ok(info) => info,
        Err(resp) => return Ok(resp),
    };

    // Validate queue is persisted (if runbook is available)
    if let Err(resp) = validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
    }

    // Collect pending item IDs and build response summaries
    let pending_items: Vec<crate::protocol::QueueItemSummary> = {
        let state = ctx.state.lock();
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
                        status: oj_storage::QueueItemStatus::Pending.to_string(),
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
        emit(&ctx.event_bus, event)?;
    }

    Ok(Response::QueueDrained {
        queue_name: queue_name.to_string(),
        items: pending_items,
    })
}

/// Handle a QueueFail request — force-fail an active item.
pub(super) fn handle_queue_fail(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
) -> Result<Response, ConnectionError> {
    // Load runbook (optional — queue may exist only in persisted state).
    let runbook_info = match load_runbook_for_queue_or_state(
        ctx,
        project_root,
        namespace,
        queue_name,
        "oj queue fail",
    ) {
        Ok(info) => info,
        Err(resp) => return Ok(resp),
    };

    // Validate queue is persisted (if runbook is available)
    if let Err(resp) = validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
    }

    // Resolve item ID (exact or prefix match)
    let resolved_id = match resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    // Validate item is Active
    {
        let st = ctx.state.lock();
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
    emit(&ctx.event_bus, event)?;

    Ok(Response::QueueFailed {
        queue_name: queue_name.to_string(),
        item_id: resolved_id,
    })
}

/// Handle a QueueDone request — force-complete an active item.
pub(super) fn handle_queue_done(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
) -> Result<Response, ConnectionError> {
    // Load runbook (optional — queue may exist only in persisted state).
    let runbook_info = match load_runbook_for_queue_or_state(
        ctx,
        project_root,
        namespace,
        queue_name,
        "oj queue done",
    ) {
        Ok(info) => info,
        Err(resp) => return Ok(resp),
    };

    // Validate queue is persisted (if runbook is available)
    if let Err(resp) = validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
    }

    // Resolve item ID (exact or prefix match)
    let resolved_id = match resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
        Ok(id) => id,
        Err(resp) => return Ok(resp),
    };

    // Validate item is Active
    {
        let st = ctx.state.lock();
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
    emit(&ctx.event_bus, event)?;

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
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    all: bool,
    dry_run: bool,
) -> Result<Response, ConnectionError> {
    // Load runbook (optional — queue may exist only in persisted state).
    let runbook_info = match load_runbook_for_queue_or_state(
        ctx,
        project_root,
        namespace,
        queue_name,
        "oj queue prune",
    ) {
        Ok(info) => info,
        Err(resp) => return Ok(resp),
    };

    // Validate queue is persisted (if runbook is available)
    if let Err(resp) = validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
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
        let state = ctx.state.lock();
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
                    status: item.status.to_string(),
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
            emit(&ctx.event_bus, event)?;
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

/// Try to load a runbook for a queue, falling back to persisted state.
///
/// Management operations (drop, retry, prune, drain, fail, done) should work on
/// persisted queues even when the runbook definition has been removed or renamed.
/// Returns `Ok(None)` when no runbook is found but the queue exists in state.
fn load_runbook_for_queue_or_state(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    command_prefix: &str,
) -> Result<Option<(oj_runbook::Runbook, PathBuf)>, Response> {
    match super::load_runbook_with_fallback(
        project_root,
        namespace,
        &ctx.state,
        |root| load_runbook_for_queue(root, queue_name),
        || {
            suggest_for_queue(
                project_root,
                queue_name,
                namespace,
                command_prefix,
                &ctx.state,
            )
        },
    ) {
        Ok(result) => Ok(Some(result)),
        Err(error_resp) => {
            // No runbook found — check if queue exists in persisted state
            let key = scoped_name(namespace, queue_name);
            let exists = ctx.state.lock().queue_items.contains_key(&key);
            if exists {
                Ok(None)
            } else {
                Err(error_resp)
            }
        }
    }
}

/// Validate that a runbook's queue definition is persisted, if present.
///
/// When `runbook_info` is `Some`, checks that the queue exists in the runbook and
/// is of type `Persisted`. When `None` (queue only in state), validation is skipped
/// since presence in state implies a persisted queue.
fn validate_persisted_queue(
    runbook_info: &Option<(oj_runbook::Runbook, PathBuf)>,
    queue_name: &str,
) -> Result<(), Response> {
    if let Some((ref runbook, _)) = runbook_info {
        match runbook.get_queue(queue_name) {
            Some(def) if def.queue_type != QueueType::Persisted => {
                return Err(Response::Error {
                    message: format!("queue '{}' is not a persisted queue", queue_name),
                });
            }
            None => {
                return Err(Response::Error {
                    message: format!("unknown queue: {}", queue_name),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

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
