// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker request handlers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::Event;
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::mutations::emit;
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
    all: bool,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    if all {
        return handle_worker_start_all(project_root, namespace, event_bus, state);
    }

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

    // Validate referenced job exists
    if runbook.get_job(&worker_def.handler.job).is_none() {
        return Ok(Response::Error {
            message: format!(
                "worker '{}' references unknown job '{}'",
                worker_name, worker_def.handler.job
            ),
        });
    }

    // Hash runbook and emit RunbookLoaded for WAL persistence
    let runbook_hash = hash_and_emit_runbook(event_bus, &runbook)?;

    // Emit WorkerStarted event
    emit(
        event_bus,
        Event::WorkerStarted {
            worker_name: worker_name.to_string(),
            project_root: project_root.to_path_buf(),
            runbook_hash,
            queue_name: worker_def.source.queue.clone(),
            concurrency: worker_def.concurrency,
            namespace: namespace.to_string(),
        },
    )?;

    Ok(Response::WorkerStarted {
        worker_name: worker_name.to_string(),
    })
}

/// Handle starting all workers defined in runbooks.
fn handle_worker_start_all(
    project_root: &Path,
    namespace: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Resolve the effective project root: prefer known root when namespace doesn't match
    let effective_root = resolve_effective_project_root(project_root, namespace, state);
    let runbook_dir = effective_root.join(".oj/runbooks");
    let names = oj_runbook::collect_all_workers(&runbook_dir)
        .unwrap_or_default()
        .into_iter()
        .map(|(name, _)| name);

    let (started, skipped) = super::collect_start_results(
        names,
        |name| handle_worker_start(&effective_root, namespace, name, false, event_bus, state),
        |resp| match resp {
            Response::WorkerStarted { worker_name } => Some(worker_name.clone()),
            _ => None,
        },
    )?;

    Ok(Response::WorkersStarted { started, skipped })
}

/// Resolve the effective project root based on namespace.
///
/// When the requested namespace differs from what would be resolved from `project_root`,
/// returns the known project root for that namespace if available.
fn resolve_effective_project_root(
    project_root: &Path,
    namespace: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> PathBuf {
    let project_namespace = oj_core::namespace::resolve_namespace(project_root);
    if !namespace.is_empty() && namespace != project_namespace {
        let known_root = {
            let st = state.lock();
            st.project_root_for_namespace(namespace)
        };
        if let Some(known) = known_root {
            return known;
        }
    }
    project_root.to_path_buf()
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
    if let Err(resp) = super::require_scoped_resource(
        state,
        namespace,
        worker_name,
        "worker",
        |s, k| s.workers.contains_key(k),
        || {
            suggest_for_worker(
                project_root,
                worker_name,
                namespace,
                "oj worker stop",
                state,
            )
        },
    ) {
        return Ok(resp);
    }

    emit(
        event_bus,
        Event::WorkerStopped {
            worker_name: worker_name.to_string(),
            namespace: namespace.to_string(),
        },
    )?;

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
    if super::scoped_exists(state, namespace, worker_name, |s, k| {
        s.workers.contains_key(k)
    }) {
        emit(
            event_bus,
            Event::WorkerStopped {
                worker_name: worker_name.to_string(),
                namespace: namespace.to_string(),
            },
        )?;
    }

    // Start with fresh runbook
    handle_worker_start(
        project_root,
        namespace,
        worker_name,
        false,
        event_bus,
        state,
    )
}

/// Handle a WorkerResize request: update concurrency at runtime.
pub(super) fn handle_worker_resize(
    worker_name: &str,
    namespace: &str,
    concurrency: u32,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Validate concurrency > 0
    if concurrency == 0 {
        return Ok(Response::Error {
            message: "concurrency must be at least 1".to_string(),
        });
    }

    // Check if worker exists and get current concurrency
    let scoped = oj_core::scoped_name(namespace, worker_name);
    let old_concurrency = match state.lock().workers.get(&scoped) {
        Some(record) => record.concurrency,
        None => {
            return Ok(Response::Error {
                message: format!("unknown worker: {}", worker_name),
            })
        }
    };

    // Emit event
    emit(
        event_bus,
        Event::WorkerResized {
            worker_name: worker_name.to_string(),
            concurrency,
            namespace: namespace.to_string(),
        },
    )?;

    Ok(Response::WorkerResized {
        worker_name: worker_name.to_string(),
        old_concurrency,
        new_concurrency: concurrency,
    })
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;

/// Hash a runbook, emit `RunbookLoaded`, and return the hash.
///
/// Combines [`hash_runbook`] with emitting the `RunbookLoaded` event, which
/// is the common pattern across worker, cron, and queue start handlers.
pub(super) fn hash_and_emit_runbook(
    event_bus: &EventBus,
    runbook: &oj_runbook::Runbook,
) -> Result<String, ConnectionError> {
    let (runbook_json, runbook_hash) = hash_runbook(runbook).map_err(ConnectionError::Internal)?;
    emit(
        event_bus,
        Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json,
        },
    )?;
    Ok(runbook_hash)
}

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
