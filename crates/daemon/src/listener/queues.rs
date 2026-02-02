// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue request handlers.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use oj_core::Event;
use oj_runbook::QueueType;
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::Response;

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
    // Load runbook containing the queue
    let runbook = match load_runbook_for_queue(project_root, queue_name) {
        Ok(rb) => rb,
        Err(e) => return Ok(Response::Error { message: e }),
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
        let scoped = if namespace.is_empty() {
            (*name).to_string()
        } else {
            format!("{}/{}", namespace, name)
        };
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

#[cfg(test)]
#[path = "queues_tests.rs"]
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
