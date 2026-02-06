// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue-to-worker integration.
//!
//! Handles waking running workers and auto-starting stopped workers
//! when queue events occur.

use std::path::Path;

use oj_core::{scoped_name, Event};

use super::super::mutations::emit;
use super::super::workers::hash_and_emit_runbook;
use super::super::ConnectionError;
use super::super::ListenCtx;

/// Find workers in the runbook that source from the given queue.
fn find_workers_for_queue<'a>(runbook: &'a oj_runbook::Runbook, queue_name: &str) -> Vec<&'a str> {
    runbook
        .workers
        .iter()
        .filter(|(_, w)| w.source.queue == queue_name)
        .map(|(name, _)| name.as_str())
        .collect()
}

/// Wake a running worker by emitting a WorkerWake event.
fn wake_running_worker(
    ctx: &ListenCtx,
    namespace: &str,
    worker_name: &str,
    queue_name: &str,
) -> Result<(), ConnectionError> {
    tracing::info!(
        queue = queue_name,
        worker = worker_name,
        "waking running worker on queue push"
    );
    emit(
        &ctx.event_bus,
        Event::WorkerWake {
            worker_name: worker_name.to_string(),
            namespace: namespace.to_string(),
        },
    )
}

/// Auto-start a stopped worker by emitting RunbookLoaded + WorkerStarted.
fn auto_start_worker(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    runbook: &oj_runbook::Runbook,
    worker_name: &str,
    queue_name: &str,
) -> Result<(), ConnectionError> {
    let Some(worker_def) = runbook.get_worker(worker_name) else {
        return Ok(());
    };
    let runbook_hash = hash_and_emit_runbook(&ctx.event_bus, runbook)?;

    emit(
        &ctx.event_bus,
        Event::WorkerStarted {
            worker_name: worker_name.to_string(),
            project_root: project_root.to_path_buf(),
            runbook_hash,
            queue_name: worker_def.source.queue.clone(),
            concurrency: worker_def.concurrency,
            namespace: namespace.to_string(),
        },
    )?;

    tracing::info!(
        queue = queue_name,
        worker = worker_name,
        "auto-started worker on queue push"
    );
    Ok(())
}

/// Wake or auto-start workers that are attached to the given queue.
///
/// For workers already running, emits `WorkerWake`. For workers that are
/// stopped or never started, emits `RunbookLoaded` + `WorkerStarted` (the
/// same events `handle_worker_start()` produces), effectively auto-starting
/// the worker on queue push.
pub(super) fn wake_attached_workers(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    runbook: &oj_runbook::Runbook,
) -> Result<(), ConnectionError> {
    let worker_names = find_workers_for_queue(runbook, queue_name);

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
            wake_running_worker(ctx, namespace, name, queue_name)?;
        } else {
            auto_start_worker(ctx, project_root, namespace, runbook, name, queue_name)?;
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

/// Emit an event and then wake attached workers.
pub(super) fn emit_and_wake_workers(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    runbook: &oj_runbook::Runbook,
    event: Event,
) -> Result<(), ConnectionError> {
    emit(&ctx.event_bus, event)?;
    wake_attached_workers(ctx, project_root, namespace, queue_name, runbook)
}
