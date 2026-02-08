// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for worker lifecycle handling (start/stop/resize)

use crate::runtime::handlers::worker::WorkerStatus;
use crate::test_helpers::{load_runbook_hash, setup_with_runbook, TestContext};
use oj_core::{Clock, Event, JobId, TimerId};
use std::collections::HashMap;

/// External queue runbook (default queue type)
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

/// Persisted queue runbook
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

// ============================================================================
// handle_worker_started tests
// ============================================================================

#[tokio::test]
async fn started_external_queue_sets_running_state() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    start_worker(&ctx, EXTERNAL_RUNBOOK, "").await;

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.status, WorkerStatus::Running);
    assert_eq!(state.queue_name, "bugs");
    assert_eq!(state.job_kind, "build");
    assert_eq!(state.concurrency, 2);
    assert!(state.active_jobs.is_empty());
    assert_eq!(state.pending_takes, 0);
}

#[tokio::test]
async fn started_persisted_queue_polls_immediately() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;

    // Push an item before starting worker
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-1".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "bug 1".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: String::new(),
        });
    });

    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);
    let events = ctx
        .runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Should return a WorkerPollComplete event (from poll_persisted_queue)
    let has_poll = events
        .iter()
        .any(|e| matches!(e, Event::WorkerPollComplete { .. }));
    assert!(has_poll, "persisted queue should trigger immediate poll");
}

#[tokio::test]
async fn started_external_queue_with_poll_sets_timer() {
    let ctx = setup_with_runbook(POLL_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, POLL_RUNBOOK);

    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Verify a poll timer was set via the scheduler
    let timer_ids = pending_timer_ids(&ctx);
    let poll_timer = TimerId::queue_poll("fixer", "");
    assert!(
        timer_ids.iter().any(|id| id == poll_timer.as_str()),
        "external queue with poll should set a periodic timer, found: {:?}",
        timer_ids
    );
}

#[tokio::test]
async fn started_error_worker_not_in_runbook() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, EXTERNAL_RUNBOOK);

    let result = ctx
        .runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "nonexistent".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await;

    assert!(result.is_err(), "should error when worker not in runbook");
}

#[tokio::test]
async fn started_restores_inflight_items_for_external_queue() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, EXTERNAL_RUNBOOK);

    // Pre-populate state as if daemon restarted with an active external queue job
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        });
        // Simulate a dispatched job with WorkerItemDispatched event
        state.apply_event(&Event::WorkerItemDispatched {
            worker_name: "fixer".to_string(),
            item_id: "ext-item-1".to_string(),
            job_id: JobId::new("pipe-ext"),
            namespace: String::new(),
        });
        // Also need a job record
        state.apply_event(&Event::JobCreated {
            id: JobId::new("pipe-ext"),
            kind: "build".to_string(),
            name: "test".to_string(),
            runbook_hash: hash.clone(),
            cwd: ctx.project_root.clone(),
            vars: HashMap::new(),
            initial_step: "init".to_string(),
            created_at_epoch_ms: 1000,
            namespace: String::new(),
            cron_name: None,
        });
    });

    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        })
        .await
        .unwrap();

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert!(
        state.inflight_items.contains("ext-item-1"),
        "external queue restart should restore inflight item IDs"
    );
    assert!(
        state.active_jobs.contains(&JobId::new("pipe-ext")),
        "should restore active job"
    );
}

// ============================================================================
// handle_worker_stopped tests
// ============================================================================

#[tokio::test]
async fn stopped_sets_status_and_clears_transient_state() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    start_worker(&ctx, EXTERNAL_RUNBOOK, "").await;

    // Simulate some in-flight state
    {
        let mut workers = ctx.runtime.worker_states.lock();
        let state = workers.get_mut("fixer").unwrap();
        state.pending_takes = 3;
        state.inflight_items.insert("item-a".to_string());
    }

    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.status, WorkerStatus::Stopped);
    assert_eq!(state.pending_takes, 0);
    assert!(state.inflight_items.is_empty());
}

#[tokio::test]
async fn stopped_nonexistent_worker_returns_ok() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;

    // Stopping a worker that was never started should not error
    let result = ctx
        .runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "ghost".to_string(),
            namespace: String::new(),
        })
        .await;

    assert!(result.is_ok(), "stopping unknown worker should succeed");
}

#[tokio::test]
async fn stopped_cancels_poll_timer() {
    let ctx = setup_with_runbook(POLL_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, POLL_RUNBOOK);

    // Start worker with poll timer
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Verify timer exists
    {
        let scheduler = ctx.runtime.scheduler();
        let sched = scheduler.lock();
        assert!(sched.has_timers(), "timer should exist before stop");
    }

    // Stop worker
    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Timer should be cancelled
    let scheduler = ctx.runtime.scheduler();
    let sched = scheduler.lock();
    assert!(
        !sched.has_timers(),
        "poll timer should be cancelled after stop"
    );
}

// ============================================================================
// handle_worker_resized tests
// ============================================================================

#[tokio::test]
async fn resized_updates_concurrency() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;

    ctx.runtime
        .handle_event(Event::WorkerResized {
            worker_name: "fixer".to_string(),
            concurrency: 5,
            namespace: String::new(),
        })
        .await
        .unwrap();

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.concurrency, 5);
}

#[tokio::test]
async fn resized_from_full_to_capacity_triggers_repoll() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);

    // Push items to queue
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-1".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "bug 1".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: String::new(),
        });
    });

    // Start worker with concurrency=1
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Simulate worker being at capacity (1 active job, concurrency=1)
    {
        let mut workers = ctx.runtime.worker_states.lock();
        let state = workers.get_mut("fixer").unwrap();
        state.active_jobs.insert(JobId::new("pipe-1"));
        state.concurrency = 1;
    }

    // Resize to 2 — going from full (1/1) to having capacity (1/2)
    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker_name: "fixer".to_string(),
            concurrency: 2,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Should trigger a repoll since we now have capacity
    let has_poll = events
        .iter()
        .any(|e| matches!(e, Event::WorkerPollComplete { .. }));
    assert!(
        has_poll,
        "resize from full to having capacity should trigger repoll"
    );
}

/// Helper: resizing while already having spare capacity should not trigger a repoll.
async fn assert_resize_with_existing_capacity_no_repoll(new_concurrency: u32) {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;

    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker_name: "fixer".to_string(),
            concurrency: new_concurrency,
            namespace: String::new(),
        })
        .await
        .unwrap();

    let has_poll = events
        .iter()
        .any(|e| matches!(e, Event::WorkerPollComplete { .. }));
    assert!(
        !has_poll,
        "resize with existing capacity should not trigger repoll"
    );
    let workers = ctx.runtime.worker_states.lock();
    assert_eq!(workers["fixer"].concurrency, new_concurrency);
}

#[tokio::test]
async fn resized_already_had_capacity_no_repoll() {
    assert_resize_with_existing_capacity_no_repoll(3).await;
}

#[tokio::test]
async fn resized_decrease_no_repoll() {
    assert_resize_with_existing_capacity_no_repoll(1).await;
}

#[tokio::test]
async fn resized_nonexistent_worker_returns_empty() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;

    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker_name: "ghost".to_string(),
            concurrency: 5,
            namespace: String::new(),
        })
        .await
        .unwrap();

    assert!(events.is_empty());
}

#[tokio::test]
async fn resized_stopped_worker_returns_empty() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;

    // Stop the worker first
    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Resize a stopped worker
    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker_name: "fixer".to_string(),
            concurrency: 5,
            namespace: String::new(),
        })
        .await
        .unwrap();

    assert!(
        events.is_empty(),
        "resizing a stopped worker should return empty events"
    );

    // Concurrency should NOT have changed (early return)
    let workers = ctx.runtime.worker_states.lock();
    assert_eq!(
        workers["fixer"].concurrency, 2,
        "stopped worker concurrency should not change"
    );
}

#[tokio::test]
async fn resized_with_pending_takes_counts_toward_capacity() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;

    // Worker has 1 active job + 1 pending take, concurrency=2 (full)
    {
        let mut workers = ctx.runtime.worker_states.lock();
        let state = workers.get_mut("fixer").unwrap();
        state.active_jobs.insert(JobId::new("pipe-1"));
        state.pending_takes = 1;
        state.concurrency = 2;
    }

    // Resize to 3: old active=2 (1 job + 1 take), was full at 2, now has capacity at 3
    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker_name: "fixer".to_string(),
            concurrency: 3,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Should repoll since we went from full (2/2) to having capacity (2/3)
    let has_poll = events
        .iter()
        .any(|e| matches!(e, Event::WorkerPollComplete { .. }));
    assert!(
        has_poll,
        "resize with pending_takes going from full to capacity should trigger repoll"
    );
}

// ============================================================================
// Namespace and cross-namespace isolation tests
// ============================================================================

#[tokio::test]
async fn started_with_namespace_stores_namespace() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    start_worker(&ctx, EXTERNAL_RUNBOOK, "myproject").await;

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("myproject/fixer").unwrap();
    assert_eq!(state.namespace, "myproject");
}

#[tokio::test]
async fn two_namespaces_same_worker_name_do_not_collide() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;

    // Start "fixer" in namespace "alpha"
    start_worker(&ctx, EXTERNAL_RUNBOOK, "alpha").await;
    // Start "fixer" in namespace "beta"
    start_worker(&ctx, EXTERNAL_RUNBOOK, "beta").await;

    // Both should exist independently
    let workers = ctx.runtime.worker_states.lock();
    assert!(
        workers.contains_key("alpha/fixer"),
        "alpha/fixer should exist"
    );
    assert!(
        workers.contains_key("beta/fixer"),
        "beta/fixer should exist"
    );
    assert_eq!(workers.len(), 2, "should have exactly 2 worker entries");

    // Both should be running
    assert_eq!(workers["alpha/fixer"].status, WorkerStatus::Running);
    assert_eq!(workers["beta/fixer"].status, WorkerStatus::Running);
    assert_eq!(workers["alpha/fixer"].namespace, "alpha");
    assert_eq!(workers["beta/fixer"].namespace, "beta");
    drop(workers);

    // Wake alpha's fixer — should not affect beta
    ctx.runtime
        .handle_event(Event::WorkerWake {
            worker_name: "fixer".to_string(),
            namespace: "alpha".to_string(),
        })
        .await
        .unwrap();

    // Stop alpha's fixer — beta should still be running
    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: "alpha".to_string(),
        })
        .await
        .unwrap();

    let workers = ctx.runtime.worker_states.lock();
    assert_eq!(
        workers["alpha/fixer"].status,
        WorkerStatus::Stopped,
        "alpha/fixer should be stopped"
    );
    assert_eq!(
        workers["beta/fixer"].status,
        WorkerStatus::Running,
        "beta/fixer should still be running"
    );
}

#[tokio::test]
async fn queue_push_only_wakes_matching_namespace_workers() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;

    // Start "fixer" in namespace "alpha" and "beta"
    start_worker(&ctx, PERSISTED_RUNBOOK, "alpha").await;
    start_worker(&ctx, PERSISTED_RUNBOOK, "beta").await;

    // Push to alpha's queue
    let events = ctx
        .runtime
        .handle_event(Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-1".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "alpha bug".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: "alpha".to_string(),
        })
        .await
        .unwrap();

    // Should emit WorkerWake for alpha's fixer only
    let wake_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::WorkerWake { .. }))
        .collect();
    assert_eq!(
        wake_events.len(),
        1,
        "should wake exactly one worker, got: {:?}",
        wake_events
    );
    match &wake_events[0] {
        Event::WorkerWake {
            worker_name,
            namespace,
        } => {
            assert_eq!(worker_name, "fixer");
            assert_eq!(namespace, "alpha");
        }
        _ => unreachable!(),
    }
}
