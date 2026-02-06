// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for worker queue polling (wake, timer)

use crate::runtime::handlers::worker::WorkerStatus;
use crate::test_helpers::{load_runbook_hash, setup_with_runbook, TestContext};
use oj_core::{Clock, Event, TimerId};

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
