// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue validation helpers.
//!
//! Centralizes runbook loading, queue existence checks, persisted-queue
//! validation, item ID resolution, and item status validation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::scoped_name;
use oj_runbook::QueueType;
use oj_storage::{MaterializedState, QueueItemStatus};

use crate::protocol::Response;

use super::super::suggest;
use super::super::ListenCtx;

/// Load a runbook that contains the given queue name.
fn load_runbook_for_queue(
    project_path: &Path,
    queue_name: &str,
) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_path.join(".oj/runbooks");
    oj_runbook::find_runbook_by_queue(&runbook_dir, queue_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no runbook found containing queue '{}'", queue_name))
}

/// Generate a "did you mean" suggestion for a queue name.
fn suggest_for_queue(
    project_path: &Path,
    queue_name: &str,
    project: &str,
    command_prefix: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> String {
    let ns = project.to_string();
    let runbook_dir = project_path.join(".oj/runbooks");
    suggest::suggest_for_resource(
        queue_name,
        project,
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

/// Load and validate a queue definition from the runbook.
///
/// Loads the runbook with project fallback, validates the queue exists,
/// and returns the runbook and effective project root.
pub(super) fn load_and_validate_queue_def(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    queue_name: &str,
    command_name: &str,
) -> Result<(oj_runbook::Runbook, PathBuf), Response> {
    let (runbook, effective_root) = super::super::load_runbook_with_fallback(
        project_path,
        project,
        &ctx.state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_path, queue_name, project, command_name, &ctx.state),
    )?;

    // Validate queue exists in the runbook
    if runbook.get_queue(queue_name).is_none() {
        return Err(Response::Error { message: format!("unknown queue: {}", queue_name) });
    }

    Ok((runbook, effective_root))
}

/// Validate that a queue is a persisted queue. Returns an error response if not.
pub(super) fn validate_persisted_queue(
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
                return Err(Response::Error { message: format!("unknown queue: {}", queue_name) });
            }
            _ => {}
        }
    }
    Ok(())
}

/// Try to load a runbook for a queue, falling back to persisted state.
///
/// Management operations (drop, retry, prune, drain, fail, done) should work on
/// persisted queues even when the runbook definition has been removed or renamed.
/// Returns `Ok(None)` when no runbook is found but the queue exists in state.
pub(super) fn load_runbook_for_queue_or_state(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    queue_name: &str,
    command_prefix: &str,
) -> Result<Option<(oj_runbook::Runbook, PathBuf)>, Response> {
    match super::super::load_runbook_with_fallback(
        project_path,
        project,
        &ctx.state,
        |root| load_runbook_for_queue(root, queue_name),
        || suggest_for_queue(project_path, queue_name, project, command_prefix, &ctx.state),
    ) {
        Ok(result) => Ok(Some(result)),
        Err(error_resp) => {
            // No runbook found — check if queue exists in persisted state
            let key = oj_core::scoped_name(project, queue_name);
            let exists = ctx.state.lock().queue_items.contains_key(&key);
            if exists {
                Ok(None)
            } else {
                Err(error_resp)
            }
        }
    }
}

/// Resolve a queue item ID by exact match or unique prefix.
///
/// Returns the full item ID on success, or an error response if the item
/// is not found or the prefix is ambiguous.
pub(super) fn resolve_queue_item_id(
    state: &Arc<Mutex<MaterializedState>>,
    project: &str,
    queue_name: &str,
    item_id: &str,
) -> Result<String, Response> {
    let state = state.lock();
    let key = scoped_name(project, queue_name);
    let items = state.queue_items.get(&key);

    // Try exact match first
    if let Some(item) = items.and_then(|items| items.iter().find(|i| i.id == item_id)) {
        return Ok(item.id.clone());
    }

    // Try prefix match
    let matches: Vec<_> = items
        .map(|items| items.iter().filter(|i| i.id.starts_with(item_id)).collect::<Vec<_>>())
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

/// Validate that a queue item is in Active status.
pub(super) fn validate_item_is_active(
    state: &Arc<Mutex<MaterializedState>>,
    project: &str,
    queue_name: &str,
    resolved_id: &str,
    action: &str,
) -> Result<(), Response> {
    let st = state.lock();
    let key = scoped_name(project, queue_name);
    let item =
        st.queue_items.get(&key).and_then(|items| items.iter().find(|i| i.id == resolved_id));
    if let Some(item) = item {
        if item.status != QueueItemStatus::Active {
            return Err(Response::Error {
                message: format!(
                    "item '{}' is {:?}, only active items can be {}",
                    resolved_id, item.status, action
                ),
            });
        }
    }
    Ok(())
}
