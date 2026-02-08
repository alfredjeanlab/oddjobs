// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for worker queue polling (wake, timer)

use crate::runtime::handlers::worker::WorkerStatus;
use crate::test_helpers::{load_runbook_hash, setup_with_runbook, TestContext};
use oj_core::{scoped_name, Clock, Event, TimerId};
use oj_storage::{QueueItem, QueueItemStatus};

/// External queue with a poll interval
const POLL_RUNBOOK: &str = r#"
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
poll = "30s"

[worker.fixer]
source = { queue = "bugs" }
handler = { job = "build" }
concurrency = 1
"#;

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

/// Start a worker by sending the WorkerStarted event through handle_event.
async fn start_worker(ctx: &TestContext, namespace: &str) {
    let hash = load_runbook_hash(ctx, POLL_RUNBOOK);
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: namespace.to_string(),
        })
        .await
        .unwrap();
}

// ============================================================================
// handle_worker_wake timer tests
// ============================================================================

#[tokio::test]
async fn wake_ensures_poll_timer_exists() {
    let ctx = setup_with_runbook(POLL_RUNBOOK).await;
    start_worker(&ctx, "").await;

    // Drain the timer set by handle_worker_started so we start clean
    {
        let scheduler = ctx.runtime.scheduler();
        let mut sched = scheduler.lock();
        ctx.clock.advance(std::time::Duration::from_secs(60));
        let _ = sched.fired_timers(ctx.clock.now());
    }

    // Verify no timers remain
    {
        let scheduler = ctx.runtime.scheduler();
        let sched = scheduler.lock();
        assert!(!sched.has_timers(), "timers should be drained");
    }

    // Send a WorkerWake event (simulates `oj worker start` on an already-running worker)
    ctx.runtime
        .handle_event(Event::WorkerWake {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // The wake should have re-established the poll timer
    let timer_ids = pending_timer_ids(&ctx);
    let poll_timer = TimerId::queue_poll("fixer", "");
    assert!(
        timer_ids.iter().any(|id| id == poll_timer.as_str()),
        "WorkerWake should ensure poll timer exists, found: {:?}",
        timer_ids
    );
}

#[tokio::test]
async fn wake_on_stopped_worker_skips_timer() {
    let ctx = setup_with_runbook(POLL_RUNBOOK).await;
    start_worker(&ctx, "").await;

    // Stop the worker
    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    assert_eq!(
        ctx.runtime
            .worker_states
            .lock()
            .get("fixer")
            .unwrap()
            .status,
        WorkerStatus::Stopped,
    );

    // Drain any remaining timers
    {
        let scheduler = ctx.runtime.scheduler();
        let mut sched = scheduler.lock();
        ctx.clock.advance(std::time::Duration::from_secs(60));
        let _ = sched.fired_timers(ctx.clock.now());
    }

    // Send a WorkerWake — should be a no-op since worker is stopped
    ctx.runtime
        .handle_event(Event::WorkerWake {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // No timer should be set
    let scheduler = ctx.runtime.scheduler();
    let sched = scheduler.lock();
    assert!(
        !sched.has_timers(),
        "wake on stopped worker should not set timer"
    );
}

// ============================================================================
// poll_persisted_queue poll_meta tests
// ============================================================================

/// Persisted queue runbook for poll_meta tests
const PERSISTED_RUNBOOK: &str = r#"
[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"

[queue.tasks]
type = "persisted"
vars = ["name"]

[worker.builder]
source = { queue = "tasks" }
handler = { job = "build" }
concurrency = 2
"#;

#[tokio::test]
async fn poll_persisted_queue_writes_poll_meta() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);
    let namespace = "testns";
    let queue_key = scoped_name(namespace, "tasks");

    // Seed queue items into state
    ctx.runtime.lock_state_mut(|s| {
        s.queue_items.insert(
            queue_key.clone(),
            vec![
                QueueItem {
                    id: "item-1".to_string(),
                    queue_name: "tasks".to_string(),
                    data: Default::default(),
                    status: QueueItemStatus::Pending,
                    worker_name: None,
                    pushed_at_epoch_ms: 0,
                    failure_count: 0,
                },
                QueueItem {
                    id: "item-2".to_string(),
                    queue_name: "tasks".to_string(),
                    data: Default::default(),
                    status: QueueItemStatus::Active,
                    worker_name: None,
                    pushed_at_epoch_ms: 0,
                    failure_count: 0,
                },
            ],
        );
    });

    // Start worker
    let worker_key = scoped_name(namespace, "builder");
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "builder".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "tasks".to_string(),
            concurrency: 2,
            namespace: namespace.to_string(),
        })
        .await
        .unwrap();

    // Advance clock so poll_meta gets a non-zero timestamp
    ctx.clock.advance(std::time::Duration::from_secs(10));

    // Poll the persisted queue
    ctx.runtime
        .poll_persisted_queue(&worker_key, "tasks", namespace)
        .unwrap();

    // Verify poll_meta was written
    let meta = ctx
        .runtime
        .lock_state(|s| s.poll_meta.get(&queue_key).cloned());
    let meta = meta.expect("poll_meta should be set after polling");
    assert_eq!(
        meta.last_item_count, 2,
        "should count all items, not just pending"
    );
    assert!(
        meta.last_polled_at_ms > 0,
        "should have a non-zero timestamp"
    );
}
