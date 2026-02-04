// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker request handlers.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::Event;
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::suggest;
use super::ConnectionError;

/// Handle a WorkerStart request.
///
/// Idempotent: always emits `WorkerStarted`. The runtime's `handle_worker_started`
/// overwrites any existing in-memory state, so repeated starts are safe and also
/// serve as a wake (triggering an initial poll).
pub(super) fn handle_worker_start(
    project_root: &Path,
    namespace: &str,
    worker_name: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook to validate worker exists.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_worker(root, worker_name),
        || {
            suggest_for_worker(
                Some(project_root),
                worker_name,
                namespace,
                "oj worker start",
                state,
            )
        },
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Validate worker definition exists
    let worker_def = match runbook.get_worker(worker_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown worker: {}", worker_name),
            })
        }
    };

    // Validate referenced queue exists
    if runbook.get_queue(&worker_def.source.queue).is_none() {
        return Ok(Response::Error {
            message: format!(
                "worker '{}' references unknown queue '{}'",
                worker_name, worker_def.source.queue
            ),
        });
    }

    // Validate referenced pipeline exists
    if runbook.get_pipeline(&worker_def.handler.pipeline).is_none() {
        return Ok(Response::Error {
            message: format!(
                "worker '{}' references unknown pipeline '{}'",
                worker_name, worker_def.handler.pipeline
            ),
        });
    }

    // Serialize and hash the runbook for WAL storage
    let (runbook_json, runbook_hash) = match hash_runbook(&runbook) {
        Ok(result) => result,
        Err(msg) => return Ok(Response::Error { message: msg }),
    };

    // Emit RunbookLoaded for WAL persistence
    let runbook_event = Event::RunbookLoaded {
        hash: runbook_hash.clone(),
        version: 1,
        runbook: runbook_json,
    };
    event_bus
        .send(runbook_event)
        .map_err(|_| ConnectionError::WalError)?;

    // Emit WorkerStarted event
    let event = Event::WorkerStarted {
        worker_name: worker_name.to_string(),
        project_root: project_root.to_path_buf(),
        runbook_hash,
        queue_name: worker_def.source.queue.clone(),
        concurrency: worker_def.concurrency,
        namespace: namespace.to_string(),
    };

    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::WorkerStarted {
        worker_name: worker_name.to_string(),
    })
}

/// Handle a WorkerStop request.
pub(super) fn handle_worker_stop(
    worker_name: &str,
    namespace: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
    project_root: Option<&Path>,
) -> Result<Response, ConnectionError> {
    // Check if worker exists in state
    let scoped = if namespace.is_empty() {
        worker_name.to_string()
    } else {
        format!("{}/{}", namespace, worker_name)
    };
    let exists = {
        let state = state.lock();
        state.workers.contains_key(&scoped)
    };
    if !exists {
        let hint = suggest_for_worker(
            project_root,
            worker_name,
            namespace,
            "oj worker stop",
            state,
        );
        return Ok(Response::Error {
            message: format!("unknown worker: {}{}", worker_name, hint),
        });
    }

    let event = Event::WorkerStopped {
        worker_name: worker_name.to_string(),
        namespace: namespace.to_string(),
    };

    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::Ok)
}

/// Handle a WorkerRestart request: stop (if running), reload runbook, start.
pub(super) fn handle_worker_restart(
    project_root: &Path,
    namespace: &str,
    worker_name: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Stop worker if it exists in state
    let scoped = if namespace.is_empty() {
        worker_name.to_string()
    } else {
        format!("{}/{}", namespace, worker_name)
    };
    let exists = {
        let state = state.lock();
        state.workers.contains_key(&scoped)
    };
    if exists {
        let stop_event = Event::WorkerStopped {
            worker_name: worker_name.to_string(),
            namespace: namespace.to_string(),
        };
        event_bus
            .send(stop_event)
            .map_err(|_| ConnectionError::WalError)?;
    }

    // Start with fresh runbook
    handle_worker_start(project_root, namespace, worker_name, event_bus, state)
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;

/// Serialize a runbook to JSON and compute its SHA256 hash.
/// Returns (runbook_json, hash_hex).
pub(super) fn hash_runbook(
    runbook: &oj_runbook::Runbook,
) -> Result<(serde_json::Value, String), String> {
    let runbook_json =
        serde_json::to_value(runbook).map_err(|e| format!("failed to serialize runbook: {}", e))?;
    let canonical = serde_json::to_string(&runbook_json)
        .map_err(|e| format!("failed to serialize runbook: {}", e))?;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(canonical.as_bytes());
    Ok((runbook_json, format!("{:x}", digest)))
}

/// Load a runbook that contains the given worker name.
fn load_runbook_for_worker(
    project_root: &Path,
    worker_name: &str,
) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_root.join(".oj/runbooks");
    oj_runbook::find_runbook_by_worker(&runbook_dir, worker_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no runbook found containing worker '{}'", worker_name))
}

/// Generate a "did you mean" suggestion for a worker name.
fn suggest_for_worker(
    project_root: Option<&Path>,
    worker_name: &str,
    namespace: &str,
    command_prefix: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> String {
    let ns = namespace.to_string();
    let root = project_root.map(|r| r.to_path_buf());
    suggest::suggest_for_resource(
        worker_name,
        namespace,
        command_prefix,
        state,
        suggest::ResourceType::Worker,
        || {
            root.map(|r| {
                oj_runbook::collect_all_workers(&r.join(".oj/runbooks"))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect()
            })
            .unwrap_or_default()
        },
        |state| {
            state
                .workers
                .values()
                .filter(|w| w.namespace == ns)
                .map(|w| w.name.clone())
                .collect()
        },
    )
}
