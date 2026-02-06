// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use oj_core::Event;

use crate::protocol::{CronEntry, Response, WorkerEntry};

use super::super::{ConnectionError, ListenCtx};
use super::{emit, PruneFlags};

/// Handle worker prune requests.
///
/// Removes all stopped workers from state by emitting WorkerDeleted events.
/// Workers are either "running" or "stopped" — all stopped workers are eligible
/// for pruning with no age threshold.
pub(crate) fn handle_worker_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = ctx.state.lock();
        for record in state_guard.workers.values() {
            // Filter by namespace if specified
            if let Some(ns) = flags.namespace {
                if record.namespace != ns {
                    continue;
                }
            }
            if record.status != "stopped" {
                skipped += 1;
                continue;
            }
            to_prune.push(WorkerEntry {
                name: record.name.clone(),
                namespace: record.namespace.clone(),
            });
        }
    }

    if !flags.dry_run {
        for entry in &to_prune {
            emit(
                &ctx.event_bus,
                Event::WorkerDeleted {
                    worker_name: entry.name.clone(),
                    namespace: entry.namespace.clone(),
                },
            )?;
        }
    }

    Ok(Response::WorkersPruned {
        pruned: to_prune,
        skipped,
    })
}

/// Handle cron prune requests.
///
/// Removes all stopped crons from state by emitting CronDeleted events.
/// Crons are either "running" or "stopped" — all stopped crons are eligible
/// for pruning with no age threshold.
pub(crate) fn handle_cron_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = ctx.state.lock();
        for record in state_guard.crons.values() {
            if record.status != "stopped" {
                skipped += 1;
                continue;
            }
            to_prune.push(CronEntry {
                name: record.name.clone(),
                namespace: record.namespace.clone(),
            });
        }
    }

    if !flags.dry_run {
        for entry in &to_prune {
            emit(
                &ctx.event_bus,
                Event::CronDeleted {
                    cron_name: entry.name.clone(),
                    namespace: entry.namespace.clone(),
                },
            )?;
        }
    }

    Ok(Response::CronsPruned {
        pruned: to_prune,
        skipped,
    })
}
