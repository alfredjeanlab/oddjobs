// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker concurrency tests (dispatch limits, repoll, stop/wake, capacity)

use super::*;

use super::worker::{
    count_dispatched, dispatched_job_ids, push_persisted_items, start_worker_and_poll,
};

/// With concurrency=2 and 3 queued items, only 2 should be dispatched immediately.
#[tokio::test]
async fn concurrency_2_dispatches_two_items_simultaneously() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 3);

    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 2).await;

    assert_eq!(count_dispatched(&events), 2, "concurrency=2 should dispatch exactly 2 items");

    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(state.active.len(), 2);
    }

    let pending_count = ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get("bugs")
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i.status == crate::storage::QueueItemStatus::Pending)
                    .count()
            })
            .unwrap_or(0)
    });
    assert_eq!(pending_count, 1, "1 item should remain Pending");
}

/// Regression guard: concurrency=1 still dispatches only 1 item.
#[tokio::test]
async fn concurrency_1_still_dispatches_one_item() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 1);
    let ctx = setup_with_runbook(&runbook).await;

    push_persisted_items(&ctx, "bugs", 3);

    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 1).await;

    assert_eq!(count_dispatched(&events), 1, "concurrency=1 should dispatch exactly 1 item");

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.active.len(), 1);
}

/// When one of two active jobs completes, the worker re-polls and fills the free slot.
#[tokio::test]
async fn job_completion_triggers_repoll_and_fills_slot() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 3);

    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    let dispatched = dispatched_job_ids(&events);
    let completed_id = &dispatched[0];

    let completion_events = ctx
        .runtime
        .handle_event(Event::JobAdvanced { id: completed_id.clone(), step: "done".to_string() })
        .await
        .unwrap();

    let mut all_events = completion_events.clone();
    for event in &completion_events {
        if matches!(event, Event::WorkerPolled { .. }) {
            let result = ctx.runtime.handle_event(event.clone()).await.unwrap();
            all_events.extend(result);
        }
    }

    assert_eq!(count_dispatched(&all_events), 1, "re-poll should dispatch the 3rd item");

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.active.len(), 2, "worker should have 2 active jobs again");
}

/// Stopping a worker marks it stopped but lets active jobs finish.
#[tokio::test]
async fn worker_stop_leaves_active_jobs_running() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 2);

    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    let dispatched = dispatched_job_ids(&events);

    let stop_events = ctx
        .runtime
        .handle_event(Event::WorkerStopped { worker: "fixer".to_string(), project: String::new() })
        .await
        .unwrap();

    // No jobs should be cancelled
    let cancelled_count = stop_events
        .iter()
        .filter(|e| matches!(e, Event::JobAdvanced { step, .. } if step == "cancelled"))
        .count();
    assert_eq!(cancelled_count, 0, "stop should not cancel active jobs");

    // Worker should be stopped but still tracking active jobs
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.status, WorkerStatus::Stopped);
    assert_eq!(state.active.len(), 2);
    for pid in &dispatched {
        assert!(state.active.contains(&pid.into()));
    }
}

/// A stopped worker should not dispatch new items on wake.
#[tokio::test]
async fn stopped_worker_does_not_dispatch_on_wake() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 1);

    // Start and dispatch first item
    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    // Stop the worker
    ctx.runtime
        .handle_event(Event::WorkerStopped { worker: "fixer".to_string(), project: String::new() })
        .await
        .unwrap();

    // Push more items and try to wake
    push_persisted_items(&ctx, "bugs", 1);
    let wake_events = ctx
        .runtime
        .handle_event(Event::WorkerWake { worker: "fixer".to_string(), project: String::new() })
        .await
        .unwrap();

    // No new dispatches should happen
    assert_eq!(count_dispatched(&wake_events), 0);
}

/// A worker at capacity should not dispatch new items even when woken.
#[tokio::test]
async fn worker_at_capacity_does_not_dispatch() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 2);

    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue: "bugs".to_string(),
            item_id: "item-extra".to_string(),
            data: vars!("title" => "extra bug"),
            pushed_at_ms: 2000,
            project: String::new(),
        });
    });

    let wake_events = ctx
        .runtime
        .handle_event(Event::WorkerWake { worker: "fixer".to_string(), project: String::new() })
        .await
        .unwrap();

    let mut all_events = Vec::new();
    for event in wake_events {
        let result = ctx.runtime.handle_event(event).await.unwrap();
        all_events.extend(result);
    }

    assert_eq!(
        count_dispatched(&all_events),
        0,
        "worker at capacity should not dispatch new items"
    );

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.active.len(), 2);
}
