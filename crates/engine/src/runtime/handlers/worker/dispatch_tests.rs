// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for worker queue dispatch (duplicate prevention)

use crate::test_helpers::{load_runbook_hash, setup_with_runbook, TestContext};
use oj_core::{Event, JobId};

/// External queue runbook
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
concurrency = 3
"#;

/// Start a worker by sending the WorkerStarted event through handle_event.
async fn start_worker(ctx: &TestContext, namespace: &str) {
    let hash = load_runbook_hash(ctx, EXTERNAL_RUNBOOK);
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 3,
            namespace: namespace.to_string(),
        })
        .await
        .unwrap();
}

// ============================================================================
// dispatch_queue_item duplicate prevention
// ============================================================================

#[tokio::test]
async fn dispatch_skips_item_already_active_in_item_job_map() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    start_worker(&ctx, "").await;

    // Simulate an active job for item "bug-1" by injecting into item_job_map
    {
        let mut workers = ctx.runtime.worker_states.lock();
        let state = workers.get_mut("fixer").unwrap();
        state
            .item_job_map
            .insert(JobId::new("existing-job"), "bug-1".to_string());
        state.active_jobs.insert(JobId::new("existing-job"));
    }

    // Send a WorkerTakeComplete for the same item — this calls dispatch_queue_item
    let events = ctx
        .runtime
        .handle_event(Event::WorkerTakeComplete {
            worker_name: "fixer".to_string(),
            item_id: "bug-1".to_string(),
            item: serde_json::json!({"id": "bug-1", "title": "duplicate"}),
            exit_code: 0,
            stderr: None,
        })
        .await
        .unwrap();

    // No JobCreated event should be emitted
    let job_created = events.iter().any(|e| matches!(e, Event::JobCreated { .. }));
    assert!(
        !job_created,
        "dispatch_queue_item should skip item already in item_job_map"
    );

    // Only the original job should remain in the worker state
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(
        state.active_jobs.len(),
        1,
        "should still have exactly one active job"
    );
    assert_eq!(
        state.item_job_map.len(),
        1,
        "should still have exactly one item mapping"
    );
}

#[tokio::test]
async fn poll_complete_skips_item_already_in_item_job_map() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    start_worker(&ctx, "").await;

    // Simulate an active job for item "bug-2" by injecting into item_job_map
    {
        let mut workers = ctx.runtime.worker_states.lock();
        let state = workers.get_mut("fixer").unwrap();
        state
            .item_job_map
            .insert(JobId::new("existing-job"), "bug-2".to_string());
        state.active_jobs.insert(JobId::new("existing-job"));
    }

    // Send a WorkerPollComplete that includes the already-dispatched item
    let events = ctx
        .runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: vec![serde_json::json!({"id": "bug-2", "title": "already active"})],
        })
        .await
        .unwrap();

    // The poll should not have dispatched a TakeQueueItem for bug-2.
    // pending_takes should remain 0 (the item was skipped).
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(
        state.pending_takes, 0,
        "poll should skip item already in item_job_map"
    );
    assert!(
        !state.inflight_items.contains("bug-2"),
        "item should not be marked inflight when already active"
    );

    // No new jobs or dispatches
    let job_created = events.iter().any(|e| matches!(e, Event::JobCreated { .. }));
    assert!(!job_created, "no new job should be created");
}

#[tokio::test]
async fn poll_complete_dispatches_new_item_not_in_map() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    start_worker(&ctx, "").await;

    // Send a WorkerPollComplete with a brand new item
    ctx.runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: vec![serde_json::json!({"id": "new-bug", "title": "fresh"})],
        })
        .await
        .unwrap();

    // The poll should have dispatched — pending_takes should be 1 (take command fired)
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(
        state.pending_takes, 1,
        "new item should be dispatched via take command"
    );
    assert!(
        state.inflight_items.contains("new-bug"),
        "new item should be marked inflight"
    );
}
