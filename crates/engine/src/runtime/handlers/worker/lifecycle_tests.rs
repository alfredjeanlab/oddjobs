// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for worker lifecycle handling (start/stop/resize/reconcile)

use crate::runtime::handlers::worker::WorkerStatus;
use crate::{RuntimeConfig, RuntimeDeps};
use oj_adapters::{FakeAgentAdapter, FakeNotifyAdapter, FakeSessionAdapter};
use oj_core::{Clock, Event, FakeClock, JobId, TimerId};
use oj_storage::{MaterializedState, QueueItemStatus};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;
use tokio::sync::mpsc;

type TestRuntime =
    crate::runtime::Runtime<FakeSessionAdapter, FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

struct TestContext {
    runtime: TestRuntime,
    clock: FakeClock,
    project_root: PathBuf,
}

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

/// Persisted queue with retry config
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

async fn setup_with_runbook(runbook_content: &str) -> TestContext {
    let dir = tempdir().unwrap();
    let dir_path = dir.keep();

    let runbook_dir = dir_path.join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(runbook_dir.join("test.toml"), runbook_content).unwrap();

    let sessions = FakeSessionAdapter::new();
    let agents = FakeAgentAdapter::new();
    let notifier = FakeNotifyAdapter::new();
    let clock = FakeClock::new();
    let (event_tx, _event_rx) = mpsc::channel(100);
    let runtime = TestRuntime::new(
        RuntimeDeps {
            sessions,
            agents,
            notifier,
            state: Arc::new(Mutex::new(MaterializedState::default())),
        },
        clock.clone(),
        RuntimeConfig {
            state_dir: dir_path.clone(),
            log_dir: dir_path.join("logs"),
        },
        event_tx,
    );

    TestContext {
        runtime,
        clock,
        project_root: dir_path,
    }
}

/// Parse a runbook, load it into cache + state, and return its hash.
fn load_runbook_hash(ctx: &TestContext, content: &str) -> String {
    let runbook = oj_runbook::parse_runbook(content).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };
    {
        let mut cache = ctx.runtime.runbook_cache.lock();
        cache.insert(hash.clone(), runbook);
    }
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::RunbookLoaded {
            hash: hash.clone(),
            version: 1,
            runbook: runbook_json,
        });
    });
    hash
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
        // Simulate a dispatched job with an item.id var
        state.apply_event(&Event::WorkerItemDispatched {
            worker_name: "fixer".to_string(),
            item_id: "ext-item-1".to_string(),
            job_id: JobId::new("pipe-ext"),
            namespace: String::new(),
        });
        // Also need a job record with the item.id var
        state.apply_event(&Event::JobCreated {
            id: JobId::new("pipe-ext"),
            kind: "build".to_string(),
            name: "test".to_string(),
            runbook_hash: hash.clone(),
            cwd: ctx.project_root.clone(),
            vars: {
                let mut m = HashMap::new();
                m.insert("item.id".to_string(), "ext-item-1".to_string());
                m
            },
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

#[tokio::test]
async fn resized_already_had_capacity_no_repoll() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;

    // Worker has 0 active jobs and concurrency=2 (already has capacity)
    // Resize to 3 — already had capacity, so no repoll needed
    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker_name: "fixer".to_string(),
            concurrency: 3,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // No repoll since we already had capacity
    let has_poll = events
        .iter()
        .any(|e| matches!(e, Event::WorkerPollComplete { .. }));
    assert!(
        !has_poll,
        "resize when already having capacity should not trigger repoll"
    );
}

#[tokio::test]
async fn resized_decrease_no_repoll() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    start_worker(&ctx, PERSISTED_RUNBOOK, "").await;

    // Worker has 0 active jobs and concurrency=2
    // Resize down to 1 — still has capacity, no state change for repoll
    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker_name: "fixer".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await
        .unwrap();

    let has_poll = events
        .iter()
        .any(|e| matches!(e, Event::WorkerPollComplete { .. }));
    assert!(
        !has_poll,
        "decreasing concurrency should not trigger repoll"
    );

    let workers = ctx.runtime.worker_states.lock();
    assert_eq!(workers["fixer"].concurrency, 1);
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
// reconcile_queue_items tests
// ============================================================================

#[tokio::test]
async fn reconcile_terminal_job_emits_queue_completion() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);

    // Set up state: worker with an active job that's already terminal.
    //
    // Event ordering matters: JobAdvanced to a terminal step removes the
    // job from workers.active_job_ids in MaterializedState.  To simulate
    // a crash where the daemon knew about the job but never emitted
    // QueueCompleted, we apply the job lifecycle events BEFORE the worker
    // record exists (so the removal is a no-op), then create the worker
    // and dispatch the item-to-job mapping.
    ctx.runtime.lock_state_mut(|state| {
        // Queue item: pushed and taken
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
        state.apply_event(&Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "item-1".to_string(),
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        });
        // Job created and already at terminal step (before worker record exists,
        // so JobAdvanced's active_job_ids removal is a no-op)
        state.apply_event(&Event::JobCreated {
            id: JobId::new("pipe-done"),
            kind: "build".to_string(),
            name: "test".to_string(),
            runbook_hash: hash.clone(),
            cwd: ctx.project_root.clone(),
            vars: {
                let mut m = HashMap::new();
                m.insert("item.id".to_string(), "item-1".to_string());
                m
            },
            initial_step: "init".to_string(),
            created_at_epoch_ms: 1000,
            namespace: String::new(),
            cron_name: None,
        });
        state.apply_event(&Event::JobAdvanced {
            id: JobId::new("pipe-done"),
            step: "done".to_string(),
        });
        // Now create worker record and dispatch mapping
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        });
        state.apply_event(&Event::WorkerItemDispatched {
            worker_name: "fixer".to_string(),
            item_id: "item-1".to_string(),
            job_id: JobId::new("pipe-done"),
            namespace: String::new(),
        });
    });

    // Restart the worker — reconciliation should detect the terminal job
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

    // After reconciliation, the queue item should be Completed
    let status = ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get("bugs")
            .and_then(|items| items.iter().find(|i| i.id == "item-1"))
            .map(|i| i.status.clone())
    });
    assert_eq!(
        status,
        Some(QueueItemStatus::Completed),
        "reconcile should complete queue item for terminal job"
    );
}

#[tokio::test]
async fn reconcile_untracked_active_item_adds_to_worker() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);

    // Set up: a queue item is Active assigned to worker, but worker's item_job_map
    // doesn't have it (e.g. daemon crashed after QueueTaken but before WorkerItemDispatched
    // was persisted). However a running job exists with matching item.id.
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-orphan".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "orphan".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: String::new(),
        });
        state.apply_event(&Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "item-orphan".to_string(),
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        });
        // Worker started but no WorkerItemDispatched in WAL
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        });
        // Job exists with matching item.id
        state.apply_event(&Event::JobCreated {
            id: JobId::new("pipe-orphan"),
            kind: "build".to_string(),
            name: "test".to_string(),
            runbook_hash: hash.clone(),
            cwd: ctx.project_root.clone(),
            vars: {
                let mut m = HashMap::new();
                m.insert("item.id".to_string(), "item-orphan".to_string());
                m
            },
            initial_step: "init".to_string(),
            created_at_epoch_ms: 1000,
            namespace: String::new(),
            cron_name: None,
        });
    });

    // Restart worker — reconciliation should detect the untracked job
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

    // Worker should now track this job
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert!(
        state.active_jobs.contains(&JobId::new("pipe-orphan")),
        "reconcile should add untracked job to worker active list"
    );
    assert_eq!(
        state.item_job_map.get(&JobId::new("pipe-orphan")),
        Some(&"item-orphan".to_string()),
        "reconcile should add item mapping"
    );
}

#[tokio::test]
async fn reconcile_orphaned_item_no_retry_goes_dead() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);

    // Set up: queue item is Active assigned to worker, but NO corresponding job exists
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-lost".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "lost".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: String::new(),
        });
        state.apply_event(&Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "item-lost".to_string(),
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        });
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        });
    });

    // Restart worker — reconciliation should detect orphaned item
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

    // With no retry config, item should go Dead
    let status = ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get("bugs")
            .and_then(|items| items.iter().find(|i| i.id == "item-lost"))
            .map(|i| i.status.clone())
    });
    assert_eq!(
        status,
        Some(QueueItemStatus::Dead),
        "orphaned item with no retry config should go Dead"
    );
}

#[tokio::test]
async fn reconcile_orphaned_item_with_retry_schedules_retry() {
    let ctx = setup_with_runbook(RETRY_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, RETRY_RUNBOOK);

    // Set up: queue item is Active assigned to worker, but NO corresponding job
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-retry".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "retry me".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: String::new(),
        });
        state.apply_event(&Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "item-retry".to_string(),
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        });
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        });
    });

    // Restart worker — reconciliation should detect orphaned item and schedule retry
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

    // Item should be Failed (not Dead) since retry is configured
    let status = ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get("bugs")
            .and_then(|items| items.iter().find(|i| i.id == "item-retry"))
            .map(|i| i.status.clone())
    });
    assert_eq!(
        status,
        Some(QueueItemStatus::Failed),
        "orphaned item with retry config should be Failed (awaiting retry)"
    );

    // A retry timer should be set
    let timer_ids = pending_timer_ids(&ctx);
    let retry_timer = TimerId::queue_retry("bugs", "item-retry");
    assert!(
        timer_ids.iter().any(|id| id == retry_timer.as_str()),
        "retry timer should be scheduled for orphaned item, found: {:?}",
        timer_ids
    );
}

#[tokio::test]
async fn reconcile_orphaned_item_exhausted_retries_goes_dead() {
    let ctx = setup_with_runbook(RETRY_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, RETRY_RUNBOOK);

    // Set up: orphaned item with failure_count already at max_attempts - 1.
    //
    // QueueItemRetry resets failure_count to 0, so we use QueueFailed +
    // QueueTaken cycles (without retry) to accumulate failure_count.
    // With max_attempts = 3, we need failure_count = 2 before reconciliation
    // so that the reconciliation QueueFailed bumps it to 3 (>= max_attempts).
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-exhausted".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "exhausted".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: String::new(),
        });
        // Cycle 1: fail (Pending→Failed, fc 0→1), then retake (Active, fc stays 1)
        state.apply_event(&Event::QueueFailed {
            queue_name: "bugs".to_string(),
            item_id: "item-exhausted".to_string(),
            error: "prior failure 1".to_string(),
            namespace: String::new(),
        });
        state.apply_event(&Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "item-exhausted".to_string(),
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        });
        // Cycle 2: fail (Active→Failed, fc 1→2), then retake (Active, fc stays 2)
        state.apply_event(&Event::QueueFailed {
            queue_name: "bugs".to_string(),
            item_id: "item-exhausted".to_string(),
            error: "prior failure 2".to_string(),
            namespace: String::new(),
        });
        state.apply_event(&Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "item-exhausted".to_string(),
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        });
        // Item is now Active with failure_count = 2, assigned to "fixer"
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        });
    });

    // Restart — reconcile should detect orphaned item with exhausted retries
    // Reconciliation emits QueueFailed (fc 2→3), then checks 3 >= 3 → Dead
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

    // With failure_count >= max_attempts (3 >= 3), should go Dead
    let status = ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get("bugs")
            .and_then(|items| items.iter().find(|i| i.id == "item-exhausted"))
            .map(|i| i.status.clone())
    });
    assert_eq!(
        status,
        Some(QueueItemStatus::Dead),
        "orphaned item that exhausted retries should go Dead"
    );
}

// ============================================================================
// handle_worker_started with namespace tests
// ============================================================================

#[tokio::test]
async fn started_with_namespace_stores_namespace() {
    let ctx = setup_with_runbook(EXTERNAL_RUNBOOK).await;
    start_worker(&ctx, EXTERNAL_RUNBOOK, "myproject").await;

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.namespace, "myproject");
}

#[tokio::test]
async fn reconcile_respects_namespace_scoping() {
    let ctx = setup_with_runbook(PERSISTED_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, PERSISTED_RUNBOOK);
    let namespace = "proj";

    // Set up: namespaced orphaned queue item
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "ns-item".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "ns bug".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: namespace.to_string(),
        });
        state.apply_event(&Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "ns-item".to_string(),
            worker_name: "fixer".to_string(),
            namespace: namespace.to_string(),
        });
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: namespace.to_string(),
        });
    });

    // Restart with namespace
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

    // Orphaned item under namespace "proj/bugs" should be Dead
    let scoped_queue = format!("{}/{}", namespace, "bugs");
    let status = ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get(&scoped_queue)
            .and_then(|items| items.iter().find(|i| i.id == "ns-item"))
            .map(|i| i.status.clone())
    });
    assert_eq!(
        status,
        Some(QueueItemStatus::Dead),
        "orphaned namespaced item should go Dead"
    );
}
