// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for worker reconciliation (queue items and active jobs after restart)

use crate::test_helpers::{load_runbook_hash, setup_with_runbook, TestContext};
use oj_core::{Clock, Event, JobId, TimerId};
use oj_storage::QueueItemStatus;
use std::collections::HashMap;

const PERSISTED_RUNBOOK: &str = r#"
[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo done"

[queue.bugs]
type = "persisted"
vars = ["title"]

[worker.fixer]
source = { queue = "bugs" }
handler = { job = "build" }
concurrency = 2
"#;

const RETRY_RUNBOOK: &str = r#"
[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo done"

[queue.bugs]
type = "persisted"
vars = ["title"]

[queue.bugs.retry]
attempts = 3
cooldown = "10s"

[worker.fixer]
source = { queue = "bugs" }
handler = { job = "build" }
concurrency = 2
"#;

const EXTERNAL_RUNBOOK: &str = r#"
[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo done"

[queue.bugs]
list = "echo '[]'"
take = "echo taken"

[worker.fixer]
source = { queue = "bugs" }
handler = { job = "build" }
concurrency = 2
"#;

// ============================================================================
// Helpers
// ============================================================================

/// Start a worker by sending WorkerStarted through handle_event (triggers reconciliation).
async fn start_worker(ctx: &TestContext, runbook: &str, namespace: &str) {
    let hash = load_runbook_hash(ctx, runbook);
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: namespace.to_string(),
        })
        .await
        .unwrap();
}

/// Apply push + take events to state for a queue item.
fn setup_queue_item(ctx: &TestContext, item_id: &str, namespace: &str) {
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: item_id.to_string(),
            data: HashMap::from([("title".to_string(), "item".to_string())]),
            pushed_at_epoch_ms: 1000,
            namespace: namespace.to_string(),
        });
        state.apply_event(&Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: item_id.to_string(),
            worker_name: "fixer".to_string(),
            namespace: namespace.to_string(),
        });
    });
}

/// Set up an orphaned queue item: pushed, taken (with optional failure cycles), worker started.
///
/// `failure_cycles` accumulates `failure_count` via QueueFailed→QueueTaken pairs before the
/// final orphaned state. Use 0 for a simple orphaned item, ≥1 for retry exhaustion scenarios.
fn setup_orphaned_item(
    ctx: &TestContext,
    hash: &str,
    item_id: &str,
    namespace: &str,
    failure_cycles: usize,
) {
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: item_id.to_string(),
            data: HashMap::from([("title".to_string(), "item".to_string())]),
            pushed_at_epoch_ms: 1000,
            namespace: namespace.to_string(),
        });
        if failure_cycles > 0 {
            for _ in 0..failure_cycles {
                state.apply_event(&Event::QueueFailed {
                    queue_name: "bugs".to_string(),
                    item_id: item_id.to_string(),
                    error: "prior failure".to_string(),
                    namespace: namespace.to_string(),
                });
                state.apply_event(&Event::QueueTaken {
                    queue_name: "bugs".to_string(),
                    item_id: item_id.to_string(),
                    worker_name: "fixer".to_string(),
                    namespace: namespace.to_string(),
                });
            }
        } else {
            state.apply_event(&Event::QueueTaken {
                queue_name: "bugs".to_string(),
                item_id: item_id.to_string(),
                worker_name: "fixer".to_string(),
                namespace: namespace.to_string(),
            });
        }
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.to_string(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: namespace.to_string(),
        });
    });
}

/// Apply WorkerStarted event to state (without going through handle_event).
fn apply_worker_started(ctx: &TestContext, hash: &str, namespace: &str) {
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.to_string(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: namespace.to_string(),
        });
    });
}

/// Apply a JobCreated event for a queue-dispatched job.
fn apply_job_with_item(ctx: &TestContext, hash: &str, job_id: &str, _item_id: &str, ns: &str) {
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::JobCreated {
            id: JobId::new(job_id),
            kind: "build".to_string(),
            name: "test".to_string(),
            runbook_hash: hash.to_string(),
            cwd: ctx.project_root.clone(),
            vars: HashMap::new(),
            initial_step: "init".to_string(),
            created_at_epoch_ms: 1000,
            namespace: ns.to_string(),
            cron_name: None,
        });
    });
}

/// Apply WorkerItemDispatched event.
fn apply_item_dispatched(ctx: &TestContext, item_id: &str, job_id: &str, namespace: &str) {
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::WorkerItemDispatched {
            worker_name: "fixer".to_string(),
            item_id: item_id.to_string(),
            job_id: JobId::new(job_id),
            namespace: namespace.to_string(),
        });
    });
}

/// Advance a job to its terminal "done" step.
fn advance_job_to_terminal(ctx: &TestContext, job_id: &str) {
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::JobAdvanced {
            id: JobId::new(job_id),
            step: "done".to_string(),
        });
    });
}

/// Get the status of a queue item.
fn queue_item_status(ctx: &TestContext, item_id: &str, namespace: &str) -> Option<QueueItemStatus> {
    let scoped_queue = if namespace.is_empty() {
        "bugs".to_string()
    } else {
        format!("{}/bugs", namespace)
    };
    ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get(&scoped_queue)
            .and_then(|items| items.iter().find(|i| i.id == item_id))
            .map(|i| i.status.clone())
    })
}

/// Collect all pending timer IDs from the scheduler.
fn pending_timer_ids(ctx: &TestContext) -> Vec<String> {
    let scheduler = ctx.runtime.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(std::time::Duration::from_secs(7200));
    let fired = sched.fired_timers(ctx.clock.now());
    fired
        .into_iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str().to_string()),
            _ => None,
        })
        .collect()
}

// ============================================================================
// reconcile_queue_items tests
// ============================================================================

#[tokio::test]
async fn reconcile_terminal_job_emits_queue_completion() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);

    // Push + take, create terminal job, then add worker + dispatch mapping.
    // Job must be created BEFORE worker so JobAdvanced's active_job_ids removal is a no-op.
    setup_queue_item(&ctx, "item-1", "");
    apply_job_with_item(&ctx, &hash, "pipe-done", "item-1", "");
    advance_job_to_terminal(&ctx, "pipe-done");
    apply_worker_started(&ctx, &hash, "");
    apply_item_dispatched(&ctx, "item-1", "pipe-done", "");

    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;
    assert_eq!(
        queue_item_status(&ctx, "item-1", ""),
        Some(QueueItemStatus::Completed),
        "reconcile should complete queue item for terminal job"
    );
}

#[tokio::test]
async fn reconcile_recovers_item_mapping_from_persisted_record() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);

    // Queue item dispatched with WorkerItemDispatched in WAL — runtime
    // item_job_map is rebuilt from the materialized WorkerRecord.
    setup_queue_item(&ctx, "item-orphan", "");
    apply_worker_started(&ctx, &hash, "");
    apply_job_with_item(&ctx, &hash, "pipe-orphan", "item-orphan", "");
    apply_item_dispatched(&ctx, "item-orphan", "pipe-orphan", "");

    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert!(
        state.active_jobs.contains(&JobId::new("pipe-orphan")),
        "reconcile should add untracked job to worker active list"
    );
    assert_eq!(
        state.item_job_map.get(&JobId::new("pipe-orphan")),
        Some(&"item-orphan".to_string()),
        "reconcile should restore item mapping from persisted record"
    );
}

/// Orphaned items (taken but no corresponding job) should go Dead after reconciliation.
async fn assert_orphaned_item_goes_dead(
    runbook: &str,
    item_id: &str,
    namespace: &str,
    failure_cycles: usize,
) {
    let ctx = setup_with_runbook(runbook).await;
    let hash = load_runbook_hash(&ctx, runbook);
    setup_orphaned_item(&ctx, &hash, item_id, namespace, failure_cycles);
    start_worker(&ctx, runbook, namespace).await;
    assert_eq!(
        queue_item_status(&ctx, item_id, namespace),
        Some(QueueItemStatus::Dead),
    );
}

#[tokio::test]
async fn reconcile_orphaned_item_no_retry_goes_dead() {
    assert_orphaned_item_goes_dead(PERSISTED_RUNBOOK, "item-lost", "", 0).await;
}

#[tokio::test]
async fn reconcile_orphaned_item_namespace_scoped_goes_dead() {
    assert_orphaned_item_goes_dead(PERSISTED_RUNBOOK, "ns-item", "proj", 0).await;
}

#[tokio::test]
async fn reconcile_orphaned_item_exhausted_retries_goes_dead() {
    assert_orphaned_item_goes_dead(RETRY_RUNBOOK, "item-exhausted", "", 2).await;
}

#[tokio::test]
async fn reconcile_orphaned_item_with_retry_schedules_retry() {
    let ctx = setup_with_runbook(RETRY_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, RETRY_RUNBOOK);
    setup_orphaned_item(&ctx, &hash, "item-retry", "", 0);
    start_worker(&ctx, RETRY_RUNBOOK, "").await;

    assert_eq!(
        queue_item_status(&ctx, "item-retry", ""),
        Some(QueueItemStatus::Failed),
        "orphaned item with retry config should be Failed (awaiting retry)"
    );
    let timer_ids = pending_timer_ids(&ctx);
    let retry_timer = TimerId::queue_retry("bugs", "item-retry");
    assert!(
        timer_ids.iter().any(|id| id == retry_timer.as_str()),
        "retry timer should be scheduled, found: {:?}",
        timer_ids
    );
}

// ============================================================================
// reconcile_active_jobs tests (runs for all queue types)
// ============================================================================

#[tokio::test]
async fn reconcile_external_queue_terminal_job_releases_slot() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, EXTERNAL_RUNBOOK);

    apply_worker_started(&ctx, &hash, "");
    apply_item_dispatched(&ctx, "ext-item-done", "pipe-done", "");
    apply_job_with_item(&ctx, &hash, "pipe-done", "ext-item-done", "");
    advance_job_to_terminal(&ctx, "pipe-done");

    start_worker(&ctx, EXTERNAL_RUNBOOK, "").await;

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert!(
        !state.active_jobs.contains(&JobId::new("pipe-done")),
        "terminal job should be removed from active_jobs after reconciliation"
    );
    assert!(
        !state.item_job_map.contains_key(&JobId::new("pipe-done")),
        "terminal job should be removed from item_job_map after reconciliation"
    );
}
