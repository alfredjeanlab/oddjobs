// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker-related runtime tests

use super::*;

const WORKER_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

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
concurrency = 1
"#;

/// Simulate the WAL replay scenario: RunbookLoaded followed by WorkerStarted.
///
/// After daemon restart, RunbookLoaded and WorkerStarted events from the WAL
/// are both processed through handle_event(). The RunbookLoaded handler must
/// populate the in-process cache so that WorkerStarted can find the runbook.
#[tokio::test]
async fn runbook_loaded_event_populates_cache_for_worker_started() {
    let ctx = setup_with_runbook(WORKER_RUNBOOK).await;

    // Parse and serialize the runbook (mimics what the listener does)
    let runbook = oj_runbook::parse_runbook(WORKER_RUNBOOK).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let runbook_hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };

    // Step 1: Process RunbookLoaded event (as WAL replay would)
    let events = ctx
        .runtime
        .handle_event(Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json,
        })
        .await
        .unwrap();
    assert!(events.is_empty(), "RunbookLoaded should not produce events");

    // Verify runbook is in cache
    {
        let cache = ctx.runtime.runbook_cache.lock();
        assert!(
            cache.contains_key(&runbook_hash),
            "RunbookLoaded should populate in-process cache"
        );
    }

    // Step 2: Process WorkerStarted event (as WAL replay would)
    // This should succeed because the cache was populated by RunbookLoaded
    let result = ctx
        .runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await;

    assert!(
        result.is_ok(),
        "WorkerStarted should succeed after RunbookLoaded: {:?}",
        result.err()
    );

    // Verify worker state was established
    let workers = ctx.runtime.worker_states.lock();
    assert!(
        workers.contains_key("fixer"),
        "Worker state should be registered"
    );
}

/// After daemon restart, WorkerStarted must restore active_jobs from
/// MaterializedState so concurrency limits are enforced.
#[tokio::test]
async fn worker_restart_restores_active_jobs_from_persisted_state() {
    let ctx = setup_with_runbook(WORKER_RUNBOOK).await;

    let runbook = oj_runbook::parse_runbook(WORKER_RUNBOOK).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let runbook_hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };

    // Populate MaterializedState as if WAL replay already ran:
    // a worker with one active job dispatched before restart.
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json.clone(),
        });
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        });
        state.apply_event(&Event::WorkerItemDispatched {
            worker_name: "fixer".to_string(),
            item_id: "item-1".to_string(),
            job_id: oj_core::JobId::new("pipe-running"),
            namespace: String::new(),
        });
    });

    // Also cache the runbook so handle_worker_started can find it
    ctx.runtime
        .handle_event(Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json,
        })
        .await
        .unwrap();

    // Now simulate the daemon re-processing WorkerStarted after restart
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Verify in-memory WorkerState has the active job restored
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").expect("worker state should exist");
    assert_eq!(
        state.active_jobs.len(),
        1,
        "active_jobs should be restored from persisted state"
    );
    assert!(
        state
            .active_jobs
            .contains(&oj_core::JobId::new("pipe-running")),
        "should contain the job that was running before restart"
    );
}

/// After daemon restart with a namespaced worker, WorkerStarted must restore
/// active_jobs using the scoped key (namespace/worker_name) from
/// MaterializedState. Regression test for queue items stuck in Active status
/// when namespace scoping was missing from the persisted state lookup.
#[tokio::test]
async fn worker_restart_restores_active_jobs_with_namespace() {
    let ctx = setup_with_runbook(WORKER_RUNBOOK).await;

    let runbook = oj_runbook::parse_runbook(WORKER_RUNBOOK).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let runbook_hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };

    let namespace = "myproject";

    // Populate MaterializedState as if WAL replay already ran:
    // a namespaced worker with one active job dispatched before restart.
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json.clone(),
        });
        state.apply_event(&Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: namespace.to_string(),
        });
        state.apply_event(&Event::WorkerItemDispatched {
            worker_name: "fixer".to_string(),
            item_id: "item-1".to_string(),
            job_id: oj_core::JobId::new("pipe-running"),
            namespace: namespace.to_string(),
        });
    });

    // Also cache the runbook so handle_worker_started can find it
    ctx.runtime
        .handle_event(Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json,
        })
        .await
        .unwrap();

    // Now simulate the daemon re-processing WorkerStarted after restart
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: namespace.to_string(),
        })
        .await
        .unwrap();

    // Verify in-memory WorkerState has the active job restored
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").expect("worker state should exist");
    assert_eq!(
        state.active_jobs.len(),
        1,
        "active_jobs should be restored from persisted state with namespace"
    );
    assert!(
        state
            .active_jobs
            .contains(&oj_core::JobId::new("pipe-running")),
        "should contain the job that was running before restart"
    );
}

/// Editing the runbook on disk after `oj worker start` should be picked up
/// on the next poll, not use the stale cached version.
#[tokio::test]
async fn worker_picks_up_runbook_edits_on_poll() {
    let ctx = setup_with_runbook(WORKER_RUNBOOK).await;

    // Parse, serialize and hash the original runbook
    let runbook = oj_runbook::parse_runbook(WORKER_RUNBOOK).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let original_hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };

    // Simulate RunbookLoaded + WorkerStarted (as daemon does)
    ctx.runtime
        .handle_event(Event::RunbookLoaded {
            hash: original_hash.clone(),
            version: 1,
            runbook: runbook_json,
        })
        .await
        .unwrap();

    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: original_hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Verify initial hash
    {
        let workers = ctx.runtime.worker_states.lock();
        assert_eq!(workers["fixer"].runbook_hash, original_hash);
    }

    // Edit the runbook on disk (change the init step command)
    let updated_runbook = WORKER_RUNBOOK.replace("echo init", "echo updated-init");
    let runbook_path = ctx.project_root.join(".oj/runbooks/test.toml");
    std::fs::write(&runbook_path, &updated_runbook).unwrap();

    // Compute the expected new hash
    let new_runbook = oj_runbook::parse_runbook(&updated_runbook).unwrap();
    let new_json = serde_json::to_value(&new_runbook).unwrap();
    let expected_new_hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&new_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };
    assert_ne!(
        original_hash, expected_new_hash,
        "hashes should differ after edit"
    );

    // Trigger a poll with an empty item list (still triggers refresh)
    let events = ctx
        .runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: vec![],
        })
        .await
        .unwrap();

    // The refresh should have emitted a RunbookLoaded event
    let has_runbook_loaded = events
        .iter()
        .any(|e| matches!(e, Event::RunbookLoaded { .. }));
    assert!(
        has_runbook_loaded,
        "WorkerPollComplete should emit RunbookLoaded when runbook changed on disk"
    );

    // Worker state should now have the new hash
    {
        let workers = ctx.runtime.worker_states.lock();
        assert_eq!(
            workers["fixer"].runbook_hash, expected_new_hash,
            "worker state should have updated runbook hash"
        );
    }

    // The new runbook should be in the cache
    {
        let cache = ctx.runtime.runbook_cache.lock();
        assert!(
            cache.contains_key(&expected_new_hash),
            "new runbook should be in cache"
        );
    }
}

// -- Concurrency > 1 tests --

/// Runbook with a persisted queue and concurrency = 2
const CONCURRENT_WORKER_RUNBOOK: &str = r#"
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

/// Helper: parse a runbook, load it into cache, and return its hash.
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

/// Helper: push N items to a persisted queue via MaterializedState events.
fn push_persisted_items(ctx: &TestContext, queue: &str, count: usize) {
    ctx.runtime.lock_state_mut(|state| {
        for i in 1..=count {
            state.apply_event(&Event::QueuePushed {
                queue_name: queue.to_string(),
                item_id: format!("item-{}", i),
                data: {
                    let mut m = HashMap::new();
                    m.insert("title".to_string(), format!("bug {}", i));
                    m
                },
                pushed_at_epoch_ms: 1000 + i as u64,
                namespace: String::new(),
            });
        }
    });
}

/// Count WorkerItemDispatched events in a list.
fn count_dispatched(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::WorkerItemDispatched { .. }))
        .count()
}

/// Collect job IDs from WorkerItemDispatched events.
fn dispatched_job_ids(events: &[Event]) -> Vec<JobId> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::WorkerItemDispatched { job_id, .. } => Some(job_id.clone()),
            _ => None,
        })
        .collect()
}

/// Helper: start worker and process the initial poll, returning all events.
async fn start_worker_and_poll(
    ctx: &TestContext,
    runbook_content: &str,
    worker_name: &str,
    concurrency: u32,
) -> Vec<Event> {
    let hash = load_runbook_hash(ctx, runbook_content);

    let start_events = ctx
        .runtime
        .handle_event(Event::WorkerStarted {
            worker_name: worker_name.to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency,
            namespace: String::new(),
        })
        .await
        .unwrap();

    let mut all_events = Vec::new();
    for event in start_events {
        let result = ctx.runtime.handle_event(event).await.unwrap();
        all_events.extend(result);
    }
    all_events
}

/// With concurrency=2 and 3 queued items, only 2 should be dispatched immediately.
#[tokio::test]
async fn concurrency_2_dispatches_two_items_simultaneously() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 3);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;

    assert_eq!(
        count_dispatched(&events),
        2,
        "concurrency=2 should dispatch exactly 2 items"
    );

    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(state.active_jobs.len(), 2);
    }

    let pending_count = ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get("bugs")
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i.status == oj_storage::QueueItemStatus::Pending)
                    .count()
            })
            .unwrap_or(0)
    });
    assert_eq!(pending_count, 1, "1 item should remain Pending");
}

/// Regression guard: concurrency=1 still dispatches only 1 item.
#[tokio::test]
async fn concurrency_1_still_dispatches_one_item() {
    let runbook_c1 = CONCURRENT_WORKER_RUNBOOK.replace("concurrency = 2", "concurrency = 1");
    let ctx = setup_with_runbook(&runbook_c1).await;

    push_persisted_items(&ctx, "bugs", 3);

    let events = start_worker_and_poll(&ctx, &runbook_c1, "fixer", 1).await;

    assert_eq!(
        count_dispatched(&events),
        1,
        "concurrency=1 should dispatch exactly 1 item"
    );

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.active_jobs.len(), 1);
}

/// When one of two active jobs completes, the worker re-polls and fills the free slot.
#[tokio::test]
async fn job_completion_triggers_repoll_and_fills_slot() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 3);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    let dispatched = dispatched_job_ids(&events);
    let completed_id = &dispatched[0];

    let completion_events = ctx
        .runtime
        .handle_event(Event::JobAdvanced {
            id: completed_id.clone(),
            step: "done".to_string(),
        })
        .await
        .unwrap();

    let mut all_events = completion_events.clone();
    for event in &completion_events {
        if matches!(event, Event::WorkerPollComplete { .. }) {
            let result = ctx.runtime.handle_event(event.clone()).await.unwrap();
            all_events.extend(result);
        }
    }

    assert_eq!(
        count_dispatched(&all_events),
        1,
        "re-poll should dispatch the 3rd item"
    );

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(
        state.active_jobs.len(),
        2,
        "worker should have 2 active jobs again"
    );
}

/// Stopping a worker marks it stopped but lets active jobs finish.
#[tokio::test]
async fn worker_stop_leaves_active_jobs_running() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 2);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    let dispatched = dispatched_job_ids(&events);

    let stop_events = ctx
        .runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
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
    assert_eq!(state.active_jobs.len(), 2);
    for pid in &dispatched {
        assert!(state.active_jobs.contains(pid));
    }
}

/// A stopped worker should not dispatch new items on wake.
#[tokio::test]
async fn stopped_worker_does_not_dispatch_on_wake() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 1);

    // Start and dispatch first item
    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    // Stop the worker
    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Push more items and try to wake
    push_persisted_items(&ctx, "bugs", 1);
    let wake_events = ctx
        .runtime
        .handle_event(Event::WorkerWake {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // No new dispatches should happen
    assert_eq!(count_dispatched(&wake_events), 0);
}

/// A worker at capacity should not dispatch new items even when woken.
#[tokio::test]
async fn worker_at_capacity_does_not_dispatch() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 2);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-extra".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "extra bug".to_string());
                m
            },
            pushed_at_epoch_ms: 2000,
            namespace: String::new(),
        });
    });

    let wake_events = ctx
        .runtime
        .handle_event(Event::WorkerWake {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
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
    assert_eq!(state.active_jobs.len(), 2);
}

/// After daemon restart, a worker with concurrency=2 and 2 active jobs
/// should restore both and not dispatch new items.
#[tokio::test]
async fn worker_restart_restores_multiple_active_jobs() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, CONCURRENT_WORKER_RUNBOOK);

    ctx.runtime.lock_state_mut(|state| {
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
            job_id: JobId::new("pipe-a"),
            namespace: String::new(),
        });
        state.apply_event(&Event::WorkerItemDispatched {
            worker_name: "fixer".to_string(),
            item_id: "item-2".to_string(),
            job_id: JobId::new("pipe-b"),
            namespace: String::new(),
        });
    });

    push_persisted_items(&ctx, "bugs", 1);

    let start_events = ctx
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

    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(
            state.active_jobs.len(),
            2,
            "should restore 2 active jobs from persisted state"
        );
        assert!(state.active_jobs.contains(&JobId::new("pipe-a")));
        assert!(state.active_jobs.contains(&JobId::new("pipe-b")));
    }

    let mut all_events = Vec::new();
    for event in start_events {
        let result = ctx.runtime.handle_event(event).await.unwrap();
        all_events.extend(result);
    }

    assert_eq!(
        count_dispatched(&all_events),
        0,
        "at capacity after restart, should not dispatch new items"
    );
}

/// When the runbook has not changed on disk, no RunbookLoaded event should be emitted.
#[tokio::test]
async fn worker_no_refresh_when_runbook_unchanged() {
    let ctx = setup_with_runbook(WORKER_RUNBOOK).await;

    let runbook = oj_runbook::parse_runbook(WORKER_RUNBOOK).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };

    ctx.runtime
        .handle_event(Event::RunbookLoaded {
            hash: hash.clone(),
            version: 1,
            runbook: runbook_json,
        })
        .await
        .unwrap();

    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Poll with empty items — runbook unchanged on disk
    let events = ctx
        .runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: vec![],
        })
        .await
        .unwrap();

    let has_runbook_loaded = events
        .iter()
        .any(|e| matches!(e, Event::RunbookLoaded { .. }));
    assert!(
        !has_runbook_loaded,
        "No RunbookLoaded should be emitted when runbook is unchanged"
    );

    // Hash should remain the same
    {
        let workers = ctx.runtime.worker_states.lock();
        assert_eq!(workers["fixer"].runbook_hash, hash);
    }
}

/// Helper: get the status of a queue item by id from materialized state.
fn queue_item_status(
    ctx: &TestContext,
    queue_name: &str,
    item_id: &str,
) -> Option<oj_storage::QueueItemStatus> {
    ctx.runtime.lock_state(|state| {
        state
            .queue_items
            .get(queue_name)
            .and_then(|items| items.iter().find(|i| i.id == item_id))
            .map(|i| i.status.clone())
    })
}

/// When a worker job completes ("done"), the queue item should transition
/// from Active to Completed.
#[tokio::test]
async fn queue_item_completed_on_job_done() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 1);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    // Verify item is Active
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(oj_storage::QueueItemStatus::Active),
        "item should be Active after dispatch"
    );

    let dispatched = dispatched_job_ids(&events);
    let job_id = &dispatched[0];

    // Complete the job
    ctx.runtime
        .handle_event(Event::JobAdvanced {
            id: job_id.clone(),
            step: "done".to_string(),
        })
        .await
        .unwrap();

    // Queue item should now be Completed
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(oj_storage::QueueItemStatus::Completed),
        "item should be Completed after job done"
    );
}

/// When a worker job fails, the queue item should transition from Active
/// to Failed (and then Dead if no retry config).
#[tokio::test]
async fn queue_item_failed_on_job_failure() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 1);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(oj_storage::QueueItemStatus::Active),
    );

    let dispatched = dispatched_job_ids(&events);
    let job_id = &dispatched[0];

    // Simulate shell failure which triggers fail_job
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: job_id.clone(),
            step: "init".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Queue item should no longer be Active
    let status = queue_item_status(&ctx, "bugs", "item-1");
    assert!(
        status != Some(oj_storage::QueueItemStatus::Active),
        "item should not be Active after job failure, got {:?}",
        status
    );
}

/// When a worker job is cancelled, the queue item should transition from
/// Active to Failed (and then Dead if no retry config).
#[tokio::test]
async fn queue_item_failed_on_job_cancel() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 1);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(oj_storage::QueueItemStatus::Active),
    );

    let dispatched = dispatched_job_ids(&events);
    let job_id = &dispatched[0];

    // Cancel the job
    ctx.runtime
        .handle_event(Event::JobCancel { id: job_id.clone() })
        .await
        .unwrap();

    // Queue item should no longer be Active
    let status = queue_item_status(&ctx, "bugs", "item-1");
    assert!(
        status != Some(oj_storage::QueueItemStatus::Active),
        "item should not be Active after job cancel, got {:?}",
        status
    );
}

// -- Duplicate dispatch prevention tests --

/// Stale WorkerPollComplete events whose items were already dispatched should
/// not create duplicate jobs. This guards against overlapping polls that
/// carry the same items when multiple QueuePushed events trigger rapid re-polls.
#[tokio::test]
async fn stale_poll_does_not_create_duplicate_jobs() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 2);

    // Start worker and dispatch both items (concurrency=2, 2 items)
    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    // Complete both jobs so active_jobs goes to 0
    let dispatched = dispatched_job_ids(&events);
    for pid in &dispatched {
        ctx.runtime
            .handle_event(Event::JobAdvanced {
                id: pid.clone(),
                step: "done".to_string(),
            })
            .await
            .unwrap();
    }

    // Verify items are Completed in state
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(oj_storage::QueueItemStatus::Completed),
    );
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-2"),
        Some(oj_storage::QueueItemStatus::Completed),
    );

    // active_jobs should be 0
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(state.active_jobs.len(), 0);
    }

    // Simulate a stale WorkerPollComplete with the same items
    // (as if a second poll was generated before the first one was processed)
    let stale_items: Vec<serde_json::Value> = vec![
        serde_json::json!({"id": "item-1", "title": "bug 1"}),
        serde_json::json!({"id": "item-2", "title": "bug 2"}),
    ];

    let stale_events = ctx
        .runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
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
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 3);

    // Dispatch first 2 items (concurrency=2)
    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    // Complete one job to free a slot
    let dispatched = dispatched_job_ids(&events);
    let completion_events = ctx
        .runtime
        .handle_event(Event::JobAdvanced {
            id: dispatched[0].clone(),
            step: "done".to_string(),
        })
        .await
        .unwrap();

    // Process re-poll from completion (should dispatch item-3)
    let mut repoll_events = Vec::new();
    for event in &completion_events {
        if matches!(event, Event::WorkerPollComplete { .. }) {
            let result = ctx.runtime.handle_event(event.clone()).await.unwrap();
            repoll_events.extend(result);
        }
    }
    assert_eq!(
        count_dispatched(&repoll_events),
        1,
        "re-poll should dispatch item-3"
    );

    // Now simulate a stale poll with all 3 original items.
    // item-1 is Completed, item-2 and item-3 are Active. No slots available.
    let stale_items: Vec<serde_json::Value> = (1..=3)
        .map(|i| serde_json::json!({"id": format!("item-{}", i), "title": format!("bug {}", i)}))
        .collect();

    let stale_events = ctx
        .runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
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

// -- External queue deduplication tests --

/// Runbook with an external queue and concurrency > 1
const EXTERNAL_CONCURRENT_RUNBOOK: &str = r#"
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

/// Overlapping polls for external queues should not dispatch the same item twice.
/// When the first poll dispatches a take command for an item, a second poll
/// with the same item should skip it because it's already in-flight.
#[tokio::test]
async fn external_queue_overlapping_polls_skip_inflight_items() {
    let ctx = setup_with_runbook(EXTERNAL_CONCURRENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, EXTERNAL_CONCURRENT_RUNBOOK);

    // Start the worker (external queue, concurrency=3)
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 3,
            namespace: String::new(),
        })
        .await
        .unwrap();

    let items = vec![
        serde_json::json!({"id": "bug-1", "title": "first bug"}),
        serde_json::json!({"id": "bug-2", "title": "second bug"}),
    ];

    // First poll: both items should be dispatched (take commands fired)
    ctx.runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: items.clone(),
        })
        .await
        .unwrap();

    // Verify inflight_items contains both items
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(state.pending_takes, 2, "should have 2 pending takes");
        assert!(
            state.inflight_items.contains("bug-1"),
            "bug-1 should be in-flight"
        );
        assert!(
            state.inflight_items.contains("bug-2"),
            "bug-2 should be in-flight"
        );
    }

    // Second poll with the same items (simulates overlapping poll):
    // should skip both because they are already in-flight
    ctx.runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: items.clone(),
        })
        .await
        .unwrap();

    // pending_takes should still be 2 (no new takes dispatched)
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(
            state.pending_takes, 2,
            "overlapping poll should not dispatch duplicate takes for in-flight items"
        );
        assert_eq!(
            state.inflight_items.len(),
            2,
            "inflight set should still have exactly 2 items"
        );
    }
}

/// After a take command fails, the item should be removed from inflight_items
/// so it can be retried on the next poll.
#[tokio::test]
async fn external_queue_take_failure_clears_inflight() {
    let ctx = setup_with_runbook(EXTERNAL_CONCURRENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, EXTERNAL_CONCURRENT_RUNBOOK);

    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 3,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Poll with one item
    ctx.runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: vec![serde_json::json!({"id": "bug-1", "title": "a bug"})],
        })
        .await
        .unwrap();

    // Verify item is in-flight
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert!(state.inflight_items.contains("bug-1"));
        assert_eq!(state.pending_takes, 1);
    }

    // Simulate take command failure
    ctx.runtime
        .handle_event(Event::WorkerTakeComplete {
            worker_name: "fixer".to_string(),
            item_id: "bug-1".to_string(),
            item: serde_json::json!({"id": "bug-1", "title": "a bug"}),
            exit_code: 1,
            stderr: Some("take failed".to_string()),
        })
        .await
        .unwrap();

    // Item should be removed from inflight so it can be retried
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert!(
            !state.inflight_items.contains("bug-1"),
            "failed take should remove item from inflight set"
        );
        assert_eq!(
            state.pending_takes, 0,
            "pending_takes should be decremented after take failure"
        );
    }
}

/// Worker stop should clear inflight_items so stale state doesn't carry over.
#[tokio::test]
async fn worker_stop_clears_inflight_items() {
    let ctx = setup_with_runbook(EXTERNAL_CONCURRENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, EXTERNAL_CONCURRENT_RUNBOOK);

    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash,
            queue_name: "bugs".to_string(),
            concurrency: 3,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Simulate in-flight items
    {
        let mut workers = ctx.runtime.worker_states.lock();
        let state = workers.get_mut("fixer").unwrap();
        state.inflight_items.insert("bug-1".to_string());
        state.inflight_items.insert("bug-2".to_string());
    }

    // Stop the worker
    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // inflight_items should be cleared
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert!(
            state.inflight_items.is_empty(),
            "worker stop should clear inflight_items"
        );
    }
}

// -- Variable namespace isolation tests --

/// Runbook with a worker that creates jobs from queue items.
/// The job declares vars = ["epic"] so fields should be mapped to epic.* and item.*
const NAMESPACED_WORKER_RUNBOOK: &str = r#"
[job.handle-epic]
vars = ["epic"]

[[job.handle-epic.step]]
name = "init"
run = "echo ${var.epic.title}"
on_done = "done"

[[job.handle-epic.step]]
name = "done"
run = "echo done"

[queue.bugs]
type = "persisted"
vars = ["title", "labels"]

[worker.fixer]
source = { queue = "bugs" }
handler = { job = "handle-epic" }
concurrency = 1
"#;

/// Worker dispatch should only create properly namespaced variable mappings:
/// - item.* (canonical namespace for queue item fields)
/// - ${first_var}.* (for backward compatibility with jobs declaring vars = ["epic"])
/// - invoke.* (system-provided invocation context)
///
/// Bare keys (like "title" without a namespace prefix) should NOT be present.
#[tokio::test]
async fn worker_dispatch_uses_namespaced_vars_only() {
    let ctx = setup_with_runbook(NAMESPACED_WORKER_RUNBOOK).await;

    // Push a queue item with title and labels fields
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "item-1".to_string(),
            data: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "Fix login bug".to_string());
                m.insert("labels".to_string(), "bug,p1".to_string());
                m
            },
            pushed_at_epoch_ms: 1000,
            namespace: String::new(),
        });
    });

    // Start worker and dispatch using the helper
    let events = start_worker_and_poll(&ctx, NAMESPACED_WORKER_RUNBOOK, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1, "should dispatch 1 item");

    // Get the dispatched job
    let job = ctx.runtime.jobs().values().next().cloned();
    assert!(job.is_some(), "job should be created");
    let job = job.unwrap();

    // Verify namespaced keys exist
    assert!(
        job.vars.contains_key("item.title"),
        "job.vars should contain item.title, got keys: {:?}",
        job.vars.keys().collect::<Vec<_>>()
    );
    assert!(
        job.vars.contains_key("item.labels"),
        "job.vars should contain item.labels"
    );
    assert!(
        job.vars.contains_key("epic.title"),
        "job.vars should contain epic.title (from first declared var)"
    );
    assert!(
        job.vars.contains_key("epic.labels"),
        "job.vars should contain epic.labels (from first declared var)"
    );

    // Verify NO bare keys (keys without a dot prefix that came from queue item fields)
    assert!(
        !job.vars.contains_key("title"),
        "job.vars should NOT contain bare 'title' key"
    );
    assert!(
        !job.vars.contains_key("labels"),
        "job.vars should NOT contain bare 'labels' key"
    );

    // All keys should have a namespace prefix (contain a dot)
    let bare_keys: Vec<_> = job.vars.keys().filter(|k| !k.contains('.')).collect();
    assert!(
        bare_keys.is_empty(),
        "job.vars should not contain bare keys, found: {:?}",
        bare_keys
    );
}

// -- pending_takes tracking tests --

/// pending_takes should count toward the concurrency limit, preventing
/// over-dispatch when external queue take commands are in-flight.
#[tokio::test]
async fn pending_takes_counted_toward_concurrency() {
    let ctx = setup_with_runbook(WORKER_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, WORKER_RUNBOOK);

    // Start the worker (external queue, concurrency=1)
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

    // Simulate an in-flight take command by setting pending_takes
    {
        let mut workers = ctx.runtime.worker_states.lock();
        let state = workers.get_mut("fixer").unwrap();
        state.pending_takes = 1;
    }

    // Fire a poll with items — should not dispatch because the pending take
    // uses the only concurrency slot
    let events = ctx
        .runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: vec![serde_json::json!({"id": "item-1", "title": "bug 1"})],
        })
        .await
        .unwrap();

    assert_eq!(
        count_dispatched(&events),
        0,
        "pending_takes should count toward concurrency limit"
    );
}

/// Worker stop should clear pending_takes so stale counts don't carry over.
#[tokio::test]
async fn worker_stop_clears_pending_takes() {
    let ctx = setup_with_runbook(WORKER_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, WORKER_RUNBOOK);

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

    // Simulate in-flight take commands
    {
        let mut workers = ctx.runtime.worker_states.lock();
        let state = workers.get_mut("fixer").unwrap();
        state.pending_takes = 2;
    }

    // Stop the worker
    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // pending_takes should be cleared
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(
            state.pending_takes, 0,
            "worker stop should clear pending_takes"
        );
    }
}
