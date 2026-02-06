// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue request handlers.

mod data_handling;
mod validation;
mod workers;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use oj_core::{scoped_name, Event};
use oj_storage::QueueItemStatus;

use crate::protocol::{QueueItemEntry, Response};

use super::mutations::emit;
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
    let (runbook, effective_root) = match validation::load_and_validate_queue_def(
        ctx,
        project_root,
        namespace,
        queue_name,
        "oj queue push",
    ) {
        Ok(r) => r,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;
    let Some(queue_def) = runbook.get_queue(queue_name) else {
        return Ok(Response::Error {
            message: format!("unknown queue: {}", queue_name),
        });
    };

    // External queues: wake workers to re-run the list command (no data needed)
    if queue_def.queue_type != oj_runbook::QueueType::Persisted {
        workers::wake_attached_workers(ctx, project_root, namespace, queue_name, &runbook)?;
        return Ok(Response::Ok);
    }

    // Validate and prepare push data
    let obj = match data_handling::validate_queue_data(&data) {
        Ok(o) => o,
        Err(resp) => return Ok(resp),
    };
    if let Err(resp) = data_handling::validate_required_fields(queue_def, obj) {
        return Ok(resp);
    }
    let final_data = data_handling::apply_defaults(queue_def, obj);

    // Deduplicate: if a pending or active item with the same data exists, return it
    if let Some(existing_id) =
        data_handling::find_duplicate_item(&ctx.state, namespace, queue_name, &final_data)
    {
        workers::wake_attached_workers(ctx, project_root, namespace, queue_name, &runbook)?;
        return Ok(Response::QueuePushed {
            queue_name: queue_name.to_string(),
            item_id: existing_id,
        });
    }

    // Generate item ID and timestamp
    let item_id = uuid::Uuid::new_v4().to_string();
    let pushed_at_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Emit event and wake workers
    workers::emit_and_wake_workers(
        ctx,
        project_root,
        namespace,
        queue_name,
        &runbook,
        Event::QueuePushed {
            queue_name: queue_name.to_string(),
            item_id: item_id.clone(),
            data: final_data,
            pushed_at_epoch_ms,
            namespace: namespace.to_string(),
        },
    )?;

    Ok(Response::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id,
    })
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
    let runbook_info = match validation::load_runbook_for_queue_or_state(
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
    if let Err(resp) = validation::validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
    }

    let resolved_id =
        match validation::resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
            Ok(id) => id,
            Err(resp) => return Ok(resp),
        };

    emit(
        &ctx.event_bus,
        Event::QueueDropped {
            queue_name: queue_name.to_string(),
            item_id: resolved_id.clone(),
            namespace: namespace.to_string(),
        },
    )?;

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
    let runbook_info = match validation::load_runbook_for_queue_or_state(
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
    if let Err(resp) = validation::validate_persisted_queue(&runbook_info, queue_name) {
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

        let resolved_id =
            match validation::resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
                Ok(id) => id,
                Err(resp) => return Ok(resp),
            };
        if let Err(resp) = validation::validate_item_is_dead_or_failed(
            &ctx.state,
            namespace,
            queue_name,
            &resolved_id,
        ) {
            return Ok(resp);
        }

        emit(
            &ctx.event_bus,
            Event::QueueItemRetry {
                queue_name: queue_name.to_string(),
                item_id: resolved_id.clone(),
                namespace: namespace.to_string(),
            },
        )?;

        // Wake workers attached to this queue (if runbook is available)
        if let Some((ref runbook, ref effective_root)) = runbook_info {
            workers::wake_attached_workers(ctx, effective_root, namespace, queue_name, runbook)?;
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
        let resolved_id =
            match validation::resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
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
                emit(
                    &ctx.event_bus,
                    Event::QueueItemRetry {
                        queue_name: queue_name.to_string(),
                        item_id: resolved_id.clone(),
                        namespace: namespace.to_string(),
                    },
                )?;
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
            workers::wake_attached_workers(ctx, effective_root, namespace, queue_name, runbook)?;
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
    let runbook_info = match validation::load_runbook_for_queue_or_state(
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
    if let Err(resp) = validation::validate_persisted_queue(&runbook_info, queue_name) {
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
        emit(
            &ctx.event_bus,
            Event::QueueDropped {
                queue_name: queue_name.to_string(),
                item_id: item.id.clone(),
                namespace: namespace.to_string(),
            },
        )?;
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
    let runbook_info = match validation::load_runbook_for_queue_or_state(
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
    if let Err(resp) = validation::validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
    }

    let resolved_id =
        match validation::resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
            Ok(id) => id,
            Err(resp) => return Ok(resp),
        };
    if let Err(resp) = validation::validate_item_is_active(
        &ctx.state,
        namespace,
        queue_name,
        &resolved_id,
        "force-failed",
    ) {
        return Ok(resp);
    }

    emit(
        &ctx.event_bus,
        Event::QueueFailed {
            queue_name: queue_name.to_string(),
            item_id: resolved_id.clone(),
            error: "force-failed via oj queue fail".to_string(),
            namespace: namespace.to_string(),
        },
    )?;

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
    let runbook_info = match validation::load_runbook_for_queue_or_state(
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
    if let Err(resp) = validation::validate_persisted_queue(&runbook_info, queue_name) {
        return Ok(resp);
    }

    let resolved_id =
        match validation::resolve_queue_item_id(&ctx.state, namespace, queue_name, item_id) {
            Ok(id) => id,
            Err(resp) => return Ok(resp),
        };
    if let Err(resp) = validation::validate_item_is_active(
        &ctx.state,
        namespace,
        queue_name,
        &resolved_id,
        "force-completed",
    ) {
        return Ok(resp);
    }

    emit(
        &ctx.event_bus,
        Event::QueueCompleted {
            queue_name: queue_name.to_string(),
            item_id: resolved_id.clone(),
            namespace: namespace.to_string(),
        },
    )?;

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
    let runbook_info = match validation::load_runbook_for_queue_or_state(
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
    if let Err(resp) = validation::validate_persisted_queue(&runbook_info, queue_name) {
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
            emit(
                &ctx.event_bus,
                Event::QueueDropped {
                    queue_name: queue_name.to_string(),
                    item_id: entry.item_id.clone(),
                    namespace: namespace.to_string(),
                },
            )?;
        }
    }

    Ok(Response::QueuesPruned {
        pruned: to_prune,
        skipped,
    })
}

#[cfg(test)]
#[path = "../queues_tests/mod.rs"]
mod tests;
