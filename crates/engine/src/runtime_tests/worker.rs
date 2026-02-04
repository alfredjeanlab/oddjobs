// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker-related runtime tests

use super::*;

const WORKER_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "init"
run = "echo init"
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[queue.bugs]
list = "echo '[]'"
take = "echo taken"

[worker.fixer]
source = { queue = "bugs" }
handler = { pipeline = "build" }
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

/// After daemon restart, WorkerStarted must restore active_pipelines from
/// MaterializedState so concurrency limits are enforced.
#[tokio::test]
async fn worker_restart_restores_active_pipelines_from_persisted_state() {
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
    // a worker with one active pipeline dispatched before restart.
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
            pipeline_id: oj_core::PipelineId::new("pipe-running"),
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

    // Verify in-memory WorkerState has the active pipeline restored
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").expect("worker state should exist");
    assert_eq!(
        state.active_pipelines.len(),
        1,
        "active_pipelines should be restored from persisted state"
    );
    assert!(
        state
            .active_pipelines
            .contains(&oj_core::PipelineId::new("pipe-running")),
        "should contain the pipeline that was running before restart"
    );
}

/// After daemon restart with a namespaced worker, WorkerStarted must restore
/// active_pipelines using the scoped key (namespace/worker_name) from
/// MaterializedState. Regression test for queue items stuck in Active status
/// when namespace scoping was missing from the persisted state lookup.
#[tokio::test]
async fn worker_restart_restores_active_pipelines_with_namespace() {
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
    // a namespaced worker with one active pipeline dispatched before restart.
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
            pipeline_id: oj_core::PipelineId::new("pipe-running"),
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

    // Verify in-memory WorkerState has the active pipeline restored
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").expect("worker state should exist");
    assert_eq!(
        state.active_pipelines.len(),
        1,
        "active_pipelines should be restored from persisted state with namespace"
    );
    assert!(
        state
            .active_pipelines
            .contains(&oj_core::PipelineId::new("pipe-running")),
        "should contain the pipeline that was running before restart"
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
[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "init"
run = "echo init"
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[queue.bugs]
type = "persisted"
vars = ["title"]

[worker.fixer]
source = { queue = "bugs" }
handler = { pipeline = "build" }
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

/// Collect pipeline IDs from WorkerItemDispatched events.
fn dispatched_pipeline_ids(events: &[Event]) -> Vec<PipelineId> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::WorkerItemDispatched { pipeline_id, .. } => Some(pipeline_id.clone()),
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
        assert_eq!(state.active_pipelines.len(), 2);
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
    assert_eq!(state.active_pipelines.len(), 1);
}

/// When one of two active pipelines completes, the worker re-polls and fills the free slot.
#[tokio::test]
async fn pipeline_completion_triggers_repoll_and_fills_slot() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 3);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    let dispatched = dispatched_pipeline_ids(&events);
    let completed_id = &dispatched[0];

    let completion_events = ctx
        .runtime
        .handle_event(Event::PipelineAdvanced {
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
        state.active_pipelines.len(),
        2,
        "worker should have 2 active pipelines again"
    );
}

/// Stopping a worker marks it stopped but lets active pipelines finish.
#[tokio::test]
async fn worker_stop_leaves_active_pipelines_running() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 2);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    let dispatched = dispatched_pipeline_ids(&events);

    let stop_events = ctx
        .runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // No pipelines should be cancelled
    let cancelled_count = stop_events
        .iter()
        .filter(|e| matches!(e, Event::PipelineAdvanced { step, .. } if step == "cancelled"))
        .count();
    assert_eq!(
        cancelled_count, 0,
        "stop should not cancel active pipelines"
    );

    // Worker should be stopped but still tracking active pipelines
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.status, WorkerStatus::Stopped);
    assert_eq!(state.active_pipelines.len(), 2);
    for pid in &dispatched {
        assert!(state.active_pipelines.contains(pid));
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
    assert_eq!(state.active_pipelines.len(), 2);
}

/// After daemon restart, a worker with concurrency=2 and 2 active pipelines
/// should restore both and not dispatch new items.
#[tokio::test]
async fn worker_restart_restores_multiple_active_pipelines() {
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
            pipeline_id: PipelineId::new("pipe-a"),
            namespace: String::new(),
        });
        state.apply_event(&Event::WorkerItemDispatched {
            worker_name: "fixer".to_string(),
            item_id: "item-2".to_string(),
            pipeline_id: PipelineId::new("pipe-b"),
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
            state.active_pipelines.len(),
            2,
            "should restore 2 active pipelines from persisted state"
        );
        assert!(state.active_pipelines.contains(&PipelineId::new("pipe-a")));
        assert!(state.active_pipelines.contains(&PipelineId::new("pipe-b")));
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

/// When a worker pipeline completes ("done"), the queue item should transition
/// from Active to Completed.
#[tokio::test]
async fn queue_item_completed_on_pipeline_done() {
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

    let dispatched = dispatched_pipeline_ids(&events);
    let pipeline_id = &dispatched[0];

    // Complete the pipeline
    ctx.runtime
        .handle_event(Event::PipelineAdvanced {
            id: pipeline_id.clone(),
            step: "done".to_string(),
        })
        .await
        .unwrap();

    // Queue item should now be Completed
    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(oj_storage::QueueItemStatus::Completed),
        "item should be Completed after pipeline done"
    );
}

/// When a worker pipeline fails, the queue item should transition from Active
/// to Failed (and then Dead if no retry config).
#[tokio::test]
async fn queue_item_failed_on_pipeline_failure() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 1);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(oj_storage::QueueItemStatus::Active),
    );

    let dispatched = dispatched_pipeline_ids(&events);
    let pipeline_id = &dispatched[0];

    // Simulate shell failure which triggers fail_pipeline
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: pipeline_id.clone(),
            step: "init".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    // Queue item should no longer be Active
    let status = queue_item_status(&ctx, "bugs", "item-1");
    assert!(
        status != Some(oj_storage::QueueItemStatus::Active),
        "item should not be Active after pipeline failure, got {:?}",
        status
    );
}

/// When a worker pipeline is cancelled, the queue item should transition from
/// Active to Failed (and then Dead if no retry config).
#[tokio::test]
async fn queue_item_failed_on_pipeline_cancel() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 1);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    assert_eq!(
        queue_item_status(&ctx, "bugs", "item-1"),
        Some(oj_storage::QueueItemStatus::Active),
    );

    let dispatched = dispatched_pipeline_ids(&events);
    let pipeline_id = &dispatched[0];

    // Cancel the pipeline
    ctx.runtime
        .handle_event(Event::PipelineCancel {
            id: pipeline_id.clone(),
        })
        .await
        .unwrap();

    // Queue item should no longer be Active
    let status = queue_item_status(&ctx, "bugs", "item-1");
    assert!(
        status != Some(oj_storage::QueueItemStatus::Active),
        "item should not be Active after pipeline cancel, got {:?}",
        status
    );
}
