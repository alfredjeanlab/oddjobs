// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker queue item lifecycle tests (done/fail/cancel) and stale poll dedup

use super::*;

use super::worker::{
    count_dispatched, dispatched_job_ids, push_persisted_items, queue_item_status,
    start_worker_and_poll,
};

/// When a worker job completes ("done"), the queue item should transition
/// from Active to Completed.
#[tokio::test]
async fn queue_item_completed_on_job_done() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 1);

    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    // Verify item is Active
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(crate::storage::QueueItemStatus::Active),
        "item should be Active after dispatch"
    );

    let dispatched = dispatched_job_ids(&events);
    let job_id = &dispatched[0];

    // Complete the job
    ctx.runtime
        .handle_event(Event::JobAdvanced { id: job_id.clone(), step: "done".to_string() })
        .await
        .unwrap();

    // Queue item should now be Completed
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(crate::storage::QueueItemStatus::Completed),
        "item should be Completed after job done"
    );
}

/// When a worker job fails, the queue item should transition from Active
/// to Failed (and then Dead if no retry config).
#[tokio::test]
async fn queue_item_failed_on_job_failure() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 1);

    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(crate::storage::QueueItemStatus::Active),
    );

    let dispatched = dispatched_job_ids(&events);
    let job_id = &dispatched[0];

    // Simulate shell failure which triggers fail_job
    ctx.runtime.handle_event(shell_fail(job_id.as_str(), "init")).await.unwrap();

    // Queue item should no longer be Active
    let status = queue_item_status(&ctx, "bugs", "item-1");
    assert!(
        status != Some(crate::storage::QueueItemStatus::Active),
        "item should not be Active after job failure, got {:?}",
        status
    );
}

/// When a worker job is cancelled, the queue item should transition from
/// Active to Failed (and then Dead if no retry config).
#[tokio::test]
async fn queue_item_failed_on_job_cancel() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 1);

    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(crate::storage::QueueItemStatus::Active),
    );

    let dispatched = dispatched_job_ids(&events);
    let job_id = &dispatched[0];

    // Cancel the job
    ctx.runtime.handle_event(Event::JobCancel { id: job_id.clone() }).await.unwrap();

    // Queue item should no longer be Active
    let status = queue_item_status(&ctx, "bugs", "item-1");
    assert!(
        status != Some(crate::storage::QueueItemStatus::Active),
        "item should not be Active after job cancel, got {:?}",
        status
    );
}

/// Stale WorkerPolled events whose items were already dispatched should
/// not create duplicate jobs. This guards against overlapping polls that
/// carry the same items when multiple QueuePushed events trigger rapid re-polls.
#[tokio::test]
async fn stale_poll_does_not_create_duplicate_jobs() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 2);

    // Start worker and dispatch both items (concurrency=2, 2 items)
    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    // Complete both jobs so active_jobs goes to 0
    let dispatched = dispatched_job_ids(&events);
    for pid in &dispatched {
        ctx.runtime
            .handle_event(Event::JobAdvanced { id: pid.clone(), step: "done".to_string() })
            .await
            .unwrap();
    }

    // Verify items are Completed in state
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(crate::storage::QueueItemStatus::Completed),
    );
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-2"),
        Some(crate::storage::QueueItemStatus::Completed),
    );

    // active_jobs should be 0
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(state.active.len(), 0);
    }

    // Simulate a stale WorkerPolled with the same items
    // (as if a second poll was generated before the first one was processed)
    let stale_items: Vec<serde_json::Value> = vec![
        serde_json::json!({"id": "item-1", "title": "bug 1"}),
        serde_json::json!({"id": "item-2", "title": "bug 2"}),
    ];

    let stale_events = ctx
        .runtime
        .handle_event(Event::WorkerPolled {
            worker: "fixer".to_string(),
            project: String::new(),
            items: stale_items,
        })
        .await
        .unwrap();

    // No new dispatches should happen: items are Completed, not Pending
    assert_eq!(
        count_dispatched(&stale_events),
        0,
        "stale poll should not create duplicate jobs for non-Pending items"
    );
}

/// When items are Active (dispatched but not yet completed), a stale poll
/// should skip them instead of creating duplicate jobs.
#[tokio::test]
async fn stale_poll_skips_active_items() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    push_persisted_items(&ctx, "bugs", 3);

    // Dispatch first 2 items (concurrency=2)
    let events = start_worker_and_poll(&ctx, &runbook, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    // Complete one job to free a slot
    let dispatched = dispatched_job_ids(&events);
    let completion_events = ctx
        .runtime
        .handle_event(Event::JobAdvanced { id: dispatched[0].clone(), step: "done".to_string() })
        .await
        .unwrap();

    // Process re-poll from completion (should dispatch item-3)
    let mut repoll_events = Vec::new();
    for event in &completion_events {
        if matches!(event, Event::WorkerPolled { .. }) {
            let result = ctx.runtime.handle_event(event.clone()).await.unwrap();
            repoll_events.extend(result);
        }
    }
    assert_eq!(count_dispatched(&repoll_events), 1, "re-poll should dispatch item-3");

    // Now simulate a stale poll with all 3 original items.
    // item-1 is Completed, item-2 and item-3 are Active. No slots available.
    let stale_items: Vec<serde_json::Value> = (1..=3)
        .map(|i| serde_json::json!({"id": format!("item-{}", i), "title": format!("bug {}", i)}))
        .collect();

    let stale_events = ctx
        .runtime
        .handle_event(Event::WorkerPolled {
            worker: "fixer".to_string(),
            project: String::new(),
            items: stale_items,
        })
        .await
        .unwrap();

    assert_eq!(
        count_dispatched(&stale_events),
        0,
        "stale poll should not dispatch: items are Active/Completed, and slots are full"
    );
}
