// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker request handlers.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

use crate::storage::MaterializedState;
use oj_core::Event;
use parking_lot::Mutex;

use super::lifecycle::{self, Stoppable};
use super::mutations::{emit, PruneFlags};
use super::suggest;
use super::{ConnectionError, ListenCtx};
use crate::event_bus::EventBus;
use crate::protocol::{Response, WorkerEntry};

pub(super) struct Worker;

impl Stoppable for Worker {
    const TYPE_NAME: &'static str = "worker";
    const APPLY_STOP: bool = false;
    fn exists(state: &MaterializedState, scoped_name: &str) -> bool {
        state.workers.contains_key(scoped_name)
    }
    fn running_names(state: &MaterializedState, project: &str) -> Vec<String> {
        state
            .workers
            .values()
            .filter(|w| w.project == project && w.status == "running")
            .map(|w| w.name.clone())
            .collect()
    }
    fn stopped_event(name: &str, project: &str) -> Event {
        Event::WorkerStopped { worker: name.to_string(), project: project.to_string() }
    }
}

/// Handle a WorkerStart request.
///
/// If the worker is already running, emits `WorkerWake` instead of `WorkerStarted`
/// to trigger a re-poll without resetting in-memory state (which would clear
/// `inflight_items` and cause duplicate dispatches for external queues).
pub(super) fn handle_worker_start(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    worker: &str,
    all: bool,
) -> Result<Response, ConnectionError> {
    if all {
        let (started, skipped) = lifecycle::handle_start_all(
            ctx,
            project_path,
            project,
            |dir| {
                oj_runbook::collect_all_workers(dir)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect()
            },
            |name| handle_worker_start(ctx, project_path, project, name, false),
            |resp| match resp {
                Response::WorkerStarted { worker } => Some(worker.clone()),
                _ => None,
            },
        )?;
        return Ok(Response::WorkersStarted { started, skipped });
    }

    // Load runbook to validate worker exists.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_path,
        project,
        &ctx.state,
        |root| load_runbook_for_worker(root, worker),
        || suggest_for_worker(Some(project_path), worker, project, "oj worker start", &ctx.state),
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_path = &effective_root;

    // Validate worker definition exists
    let worker_def = match runbook.get_worker(worker) {
        Some(def) => def,
        None => return Ok(Response::Error { message: format!("unknown worker: {}", worker) }),
    };

    // Validate referenced queue exists
    if runbook.get_queue(&worker_def.source.queue).is_none() {
        return Ok(Response::Error {
            message: format!(
                "worker '{}' references unknown queue '{}'",
                worker, worker_def.source.queue
            ),
        });
    }

    // Validate referenced job exists
    if runbook.get_job(&worker_def.run.job).is_none() {
        return Ok(Response::Error {
            message: format!("worker '{}' references unknown job '{}'", worker, worker_def.run.job),
        });
    }

    // If the worker is already running, emit WorkerWake instead of WorkerStarted
    // to trigger a re-poll without resetting in-memory state.
    let scoped = oj_core::scoped_name(project, worker);
    let already_running =
        ctx.state.lock().workers.get(&scoped).map(|w| w.status == "running").unwrap_or(false);

    if already_running {
        emit(
            &ctx.event_bus,
            Event::WorkerWake { worker: worker.to_string(), project: project.to_string() },
        )?;
        return Ok(Response::WorkerStarted { worker: worker.to_string() });
    }

    // Hash runbook and emit RunbookLoaded for WAL persistence
    let runbook_hash = hash_and_emit_runbook(&ctx.event_bus, &runbook)?;

    // Emit WorkerStarted event
    let event = Event::WorkerStarted {
        worker: worker.to_string(),
        project_path: project_path.to_path_buf(),
        runbook_hash,
        queue: worker_def.source.queue.clone(),
        concurrency: worker_def.concurrency,
        project: project.to_string(),
    };
    emit(&ctx.event_bus, event.clone())?;

    {
        let mut state = ctx.state.lock();
        state.apply_event(&event);
    }

    Ok(Response::WorkerStarted { worker: worker.to_string() })
}

/// Handle a WorkerStop request.
pub(super) fn handle_worker_stop(
    ctx: &ListenCtx,
    worker_name: &str,
    project: &str,
    project_path: Option<&Path>,
    all: bool,
) -> Result<Response, ConnectionError> {
    lifecycle::handle_stop::<Worker>(
        ctx,
        worker_name,
        project,
        all,
        || suggest_for_worker(project_path, worker_name, project, "oj worker stop", &ctx.state),
        |stopped, skipped| Response::WorkersStopped { stopped, skipped },
    )
}

/// Handle a WorkerRestart request: stop (if running), reload runbook, start.
pub(super) fn handle_worker_restart(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    worker_name: &str,
) -> Result<Response, ConnectionError> {
    lifecycle::handle_restart::<Worker>(ctx, project, worker_name, || {
        handle_worker_start(ctx, project_path, project, worker_name, false)
    })
}

/// Handle a WorkerResize request: update concurrency at runtime.
pub(super) fn handle_worker_resize(
    ctx: &ListenCtx,
    worker: &str,
    project: &str,
    concurrency: u32,
) -> Result<Response, ConnectionError> {
    if concurrency == 0 {
        return Ok(Response::Error { message: "concurrency must be at least 1".to_string() });
    }

    let scoped = oj_core::scoped_name(project, worker);
    let old_concurrency = match ctx.state.lock().workers.get(&scoped) {
        Some(record) => record.concurrency,
        None => return Ok(Response::Error { message: format!("unknown worker: {}", worker) }),
    };

    emit(
        &ctx.event_bus,
        Event::WorkerResized {
            worker: worker.to_string(),
            concurrency,
            project: project.to_string(),
        },
    )?;

    Ok(Response::WorkerResized {
        worker: worker.to_string(),
        old_concurrency,
        new_concurrency: concurrency,
    })
}

/// Handle a WorkerPrune request.
pub(super) fn handle_worker_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    lifecycle::handle_prune(
        ctx,
        flags,
        |s, ns| {
            let mut entries = Vec::new();
            let mut skipped = 0;
            for w in s.workers.values() {
                if ns.is_some_and(|n| w.project != n) {
                    continue;
                }
                if w.status != "stopped" {
                    skipped += 1;
                    continue;
                }
                entries.push(WorkerEntry::from(w));
            }
            (entries, skipped)
        },
        |e| Event::WorkerDeleted { worker: e.name.clone(), project: e.project.clone() },
        |pruned, skipped| Response::WorkersPruned { pruned, skipped },
    )
}

fn suggest_for_worker(
    root: Option<&Path>,
    name: &str,
    ns: &str,
    cmd: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> String {
    let ns_owned = ns.to_string();
    let root = root.map(|r| r.to_path_buf());
    suggest::suggest_for_resource(
        name,
        ns,
        cmd,
        state,
        suggest::ResourceType::Worker,
        || {
            root.map(|r| {
                oj_runbook::collect_all_workers(&r.join(".oj/runbooks"))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect()
            })
            .unwrap_or_default()
        },
        move |s| {
            s.workers.values().filter(|w| w.project == ns_owned).map(|w| w.name.clone()).collect()
        },
    )
}

fn load_runbook_for_worker(
    project_path: &Path,
    worker_name: &str,
) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_path.join(".oj/runbooks");
    oj_runbook::find_runbook_by_worker(&runbook_dir, worker_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no runbook found containing worker '{}'", worker_name))
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;

/// Hash a runbook, emit `RunbookLoaded`, and return the hash.
pub(super) fn hash_and_emit_runbook(
    event_bus: &EventBus,
    runbook: &oj_runbook::Runbook,
) -> Result<String, ConnectionError> {
    let (runbook_json, runbook_hash) = hash_runbook(runbook).map_err(ConnectionError::Internal)?;
    emit(
        event_bus,
        Event::RunbookLoaded { hash: runbook_hash.clone(), version: 1, runbook: runbook_json },
    )?;
    Ok(runbook_hash)
}

/// Serialize a runbook to JSON and compute its SHA256 hash.
pub(super) fn hash_runbook(
    runbook: &oj_runbook::Runbook,
) -> Result<(serde_json::Value, String), String> {
    let runbook_json =
        serde_json::to_value(runbook).map_err(|e| format!("failed to serialize runbook: {}", e))?;
    let canonical = serde_json::to_string(&runbook_json)
        .map_err(|e| format!("failed to serialize runbook: {}", e))?;
    let digest = Sha256::digest(canonical.as_bytes());
    Ok((runbook_json, format!("{:x}", digest)))
}
