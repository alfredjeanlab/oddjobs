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

/// Runbook with a persisted queue and concurrency = 2
pub(super) const CONCURRENT_WORKER_RUNBOOK: &str = r#"
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

pub(super) use crate::test_helpers::load_runbook_hash;

/// Helper: push N items to a persisted queue via MaterializedState events.
pub(super) fn push_persisted_items(ctx: &TestContext, queue: &str, count: usize) {
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
pub(super) fn count_dispatched(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::WorkerItemDispatched { .. }))
        .count()
}

/// Collect job IDs from WorkerItemDispatched events.
pub(super) fn dispatched_job_ids(events: &[Event]) -> Vec<JobId> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::WorkerItemDispatched { job_id, .. } => Some(job_id.clone()),
            _ => None,
        })
        .collect()
}

/// Helper: start worker and process the initial poll, returning all events.
pub(super) async fn start_worker_and_poll(
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

/// Helper: get the status of a queue item by id from materialized state.
pub(super) fn queue_item_status(
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

/// Simulate the WAL replay scenario: RunbookLoaded followed by WorkerStarted.
///
/// After daemon restart, RunbookLoaded and WorkerStarted events from the WAL
/// are both processed through handle_event(). The RunbookLoaded handler must
/// populate the in-process cache so that WorkerStarted can find the runbook.
#[tokio::test]
async fn runbook_loaded_event_populates_cache_for_worker_started() {
    let ctx = setup_with_runbook(WORKER_RUNBOOK).await;

    // Parse and serialize the runbook (mimics what the listener does)
    let (runbook_json, runbook_hash) = hash_runbook(WORKER_RUNBOOK);

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

    let (runbook_json, runbook_hash) = hash_runbook(WORKER_RUNBOOK);

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

    let (runbook_json, runbook_hash) = hash_runbook(WORKER_RUNBOOK);

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
    // (key is now scoped: "myproject/fixer")
    let workers = ctx.runtime.worker_states.lock();
    let state = workers
        .get("myproject/fixer")
        .expect("worker state should exist");
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
    let (runbook_json, original_hash) = hash_runbook(WORKER_RUNBOOK);

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
    let (_, expected_new_hash) = hash_runbook(&updated_runbook);
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

    let (runbook_json, hash) = hash_runbook(WORKER_RUNBOOK);

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
