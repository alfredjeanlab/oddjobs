// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for worker reconciliation (queue items after restart)

use crate::engine::test_helpers::{
    load_runbook_hash, setup_with_runbook, worker_started, TestContext,
};
use crate::storage::QueueItemStatus;
use oj_core::{Event, JobId, OwnerId, TimerId};
use std::collections::HashMap;

const PERSISTED_RUNBOOK: &str = r#"
[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = { step = "done" }

[[job.build.step]]
name = "done"
run = "echo done"

[queue.bugs]
type = "persisted"
vars = ["title"]

[worker.fixer]
run = { job = "build" }
source = { queue = "bugs" }
concurrency = 2
"#;

const RETRY_RUNBOOK: &str = r#"
[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = { step = "done" }

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
run = { job = "build" }
source = { queue = "bugs" }
concurrency = 2
"#;

/// Start a worker by sending WorkerStarted through handle_event (triggers reconciliation).
async fn start_worker(ctx: &TestContext, runbook: &str, project: &str) {
    let hash = load_runbook_hash(ctx, runbook);
    ctx.runtime
        .handle_event(worker_started("fixer", &ctx.project_path, &hash, "bugs", 2, project))
        .await
        .unwrap();
}

/// Set up an orphaned queue item: pushed, taken (with optional failure cycles), worker started.
///
/// `failure_cycles` accumulates `failures` via QueueFailed->QueueTaken pairs before the
/// final orphaned state. Use 0 for a simple orphaned item, >=1 for retry exhaustion scenarios.
fn setup_orphaned_item(
    ctx: &TestContext,
    hash: &str,
    item_id: &str,
    project: &str,
    failure_cycles: usize,
) {
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue: "bugs".to_string(),
            item_id: item_id.to_string(),
            data: HashMap::from([("title".to_string(), "item".to_string())]),
            pushed_at_ms: 1000,
            project: project.to_string(),
        });
        if failure_cycles > 0 {
            for _ in 0..failure_cycles {
                state.apply_event(&Event::QueueFailed {
                    queue: "bugs".to_string(),
                    item_id: item_id.to_string(),
                    error: "prior failure".to_string(),
                    project: project.to_string(),
                });
                state.apply_event(&Event::QueueTaken {
                    queue: "bugs".to_string(),
                    item_id: item_id.to_string(),
                    worker: "fixer".to_string(),
                    project: project.to_string(),
                });
            }
        } else {
            state.apply_event(&Event::QueueTaken {
                queue: "bugs".to_string(),
                item_id: item_id.to_string(),
                worker: "fixer".to_string(),
                project: project.to_string(),
            });
        }
        state.apply_event(&worker_started("fixer", &ctx.project_path, hash, "bugs", 2, project));
    });
}

/// Get the status of a queue item.
fn queue_item_status(ctx: &TestContext, item_id: &str, project: &str) -> Option<QueueItemStatus> {
    let scoped_queue =
        if project.is_empty() { "bugs".to_string() } else { format!("{}/bugs", project) };
    ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get(&scoped_queue)
            .and_then(|items| items.iter().find(|i| i.id == item_id))
            .map(|i| i.status.clone())
    })
}

#[tokio::test]
async fn reconcile_recovers_item_mapping_from_persisted_record() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);

    // Queue item dispatched with WorkerDispatched in WAL — runtime
    // item_owners is rebuilt from the materialized WorkerRecord.
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue: "bugs".to_string(),
            item_id: "item-orphan".to_string(),
            data: HashMap::from([("title".to_string(), "item".to_string())]),
            pushed_at_ms: 1000,
            project: String::new(),
        });
        state.apply_event(&Event::QueueTaken {
            queue: "bugs".to_string(),
            item_id: "item-orphan".to_string(),
            worker: "fixer".to_string(),
            project: String::new(),
        });
        state.apply_event(&worker_started("fixer", &ctx.project_path, &hash, "bugs", 2, ""));
        state.apply_event(&Event::JobCreated {
            id: JobId::new("job-orphan"),
            kind: "build".to_string(),
            name: "test".to_string(),
            runbook_hash: hash.clone(),
            cwd: ctx.project_path.clone(),
            vars: HashMap::new(),
            initial_step: "init".to_string(),
            created_at_ms: 1000,
            project: String::new(),
            cron: None,
        });
        state.apply_event(&Event::WorkerDispatched {
            worker: "fixer".to_string(),
            item_id: "item-orphan".to_string(),
            owner: JobId::new("job-orphan").into(),
            project: String::new(),
        });
    });

    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    let owner: OwnerId = JobId::new("job-orphan").into();
    assert!(
        state.active.contains(&owner),
        "reconcile should add untracked job to worker active list"
    );
    assert_eq!(
        state.items.get(&owner),
        Some(&"item-orphan".to_string()),
        "reconcile should restore item mapping from persisted record"
    );
}

#[yare::parameterized(
    no_retry          = { PERSISTED_RUNBOOK, "item-lost",      "",     0 },
    namespace_scoped  = { PERSISTED_RUNBOOK, "ns-item",        "proj", 0 },
    exhausted_retries = { RETRY_RUNBOOK,     "item-exhausted", "",     2 },
)]
fn reconcile_orphaned_item_goes_dead(
    runbook: &str,
    item_id: &str,
    project: &str,
    failure_cycles: usize,
) {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let ctx = setup_with_runbook(runbook).await;
        let hash = load_runbook_hash(&ctx, runbook);
        setup_orphaned_item(&ctx, &hash, item_id, project, failure_cycles);
        start_worker(&ctx, runbook, project).await;
        assert_eq!(queue_item_status(&ctx, item_id, project), Some(QueueItemStatus::Dead));
    });
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
    let timer_ids = ctx.pending_timer_ids();
    let retry_timer = TimerId::queue_retry("bugs", "item-retry");
    assert!(
        timer_ids.iter().any(|id| id == retry_timer.as_str()),
        "retry timer should be scheduled, found: {:?}",
        timer_ids
    );
}
