// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker-related runtime tests

use super::*;

pub(super) use crate::test_helpers::load_runbook_hash;

/// Helper: push N items to a persisted queue via MaterializedState events.
pub(super) fn push_persisted_items(ctx: &TestContext, queue: &str, count: usize) {
    ctx.runtime.lock_state_mut(|state| {
        for i in 1..=count {
            state.apply_event(&Event::QueuePushed {
                queue: queue.to_string(),
                item_id: format!("item-{}", i),
                data: vars!("title" => format!("bug {}", i)),
                pushed_at_ms: 1000 + i as u64,
                project: String::new(),
            });
        }
    });
}

/// Count WorkerDispatched events in a list.
pub(super) fn count_dispatched(events: &[Event]) -> usize {
    events.iter().filter(|e| matches!(e, Event::WorkerDispatched { .. })).count()
}

/// Collect job IDs from WorkerDispatched events.
pub(super) fn dispatched_job_ids(events: &[Event]) -> Vec<JobId> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::WorkerDispatched { owner, .. } => owner.as_job().cloned(),
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
        .handle_event(worker_started(
            worker_name,
            &ctx.project_path,
            &hash,
            "bugs",
            concurrency,
            "",
        ))
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
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;

    // Parse and serialize the runbook (mimics what the listener does)
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

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
        .handle_event(worker_started("fixer", &ctx.project_path, &runbook_hash, "bugs", 1, ""))
        .await;

    assert!(result.is_ok(), "WorkerStarted should succeed after RunbookLoaded: {:?}", result.err());

    // Verify worker state was established
    let workers = ctx.runtime.worker_states.lock();
    assert!(workers.contains_key("fixer"), "Worker state should be registered");
}

/// After daemon restart, WorkerStarted must restore active_jobs from
/// MaterializedState so concurrency limits are enforced.
#[tokio::test]
async fn worker_restart_restores_active_jobs_from_persisted_state() {
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;

    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    // Populate MaterializedState as if WAL replay already ran:
    // a worker with one active job dispatched before restart.
    let ws = worker_started("fixer", &ctx.project_path, &runbook_hash, "bugs", 1, "");
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json.clone(),
        });
        state.apply_event(&ws);
        state.apply_event(&Event::WorkerDispatched {
            worker: "fixer".to_string(),
            item_id: "item-1".to_string(),
            owner: oj_core::JobId::new("job-running").into(),
            project: String::new(),
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
        .handle_event(worker_started("fixer", &ctx.project_path, &runbook_hash, "bugs", 1, ""))
        .await
        .unwrap();

    // Verify in-memory WorkerState has the active job restored
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").expect("worker state should exist");
    assert_eq!(state.active.len(), 1, "active_jobs should be restored from persisted state");
    assert!(
        state.active.contains(&oj_core::OwnerId::Job(oj_core::JobId::new("job-running"))),
        "should contain the job that was running before restart"
    );
}

/// After daemon restart with a namespaced worker, WorkerStarted must restore
/// active_jobs using the scoped key (project/worker_name) from
/// MaterializedState. Regression test for queue items stuck in Active status
/// when project scoping was missing from the persisted state lookup.
#[tokio::test]
async fn worker_restart_restores_active_jobs_with_namespace() {
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;

    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    let project = "myproject";

    // Populate MaterializedState as if WAL replay already ran:
    // a namespaced worker with one active job dispatched before restart.
    let ws = worker_started("fixer", &ctx.project_path, &runbook_hash, "bugs", 1, project);
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json.clone(),
        });
        state.apply_event(&ws);
        state.apply_event(&Event::WorkerDispatched {
            worker: "fixer".to_string(),
            item_id: "item-1".to_string(),
            owner: oj_core::JobId::new("job-running").into(),
            project: project.to_string(),
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
        .handle_event(worker_started("fixer", &ctx.project_path, &runbook_hash, "bugs", 1, project))
        .await
        .unwrap();

    // Verify in-memory WorkerState has the active job restored
    // (key is now scoped: "myproject/fixer")
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("myproject/fixer").expect("worker state should exist");
    assert_eq!(
        state.active.len(),
        1,
        "active_jobs should be restored from persisted state with project"
    );
    assert!(
        state.active.contains(&oj_core::OwnerId::Job(oj_core::JobId::new("job-running"))),
        "should contain the job that was running before restart"
    );
}

/// Editing the runbook on disk after `oj worker start` should be picked up
/// on the next poll, not use the stale cached version.
#[tokio::test]
async fn worker_picks_up_runbook_edits_on_poll() {
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;

    // Parse, serialize and hash the original runbook
    let (runbook_json, original_hash) = hash_runbook(&runbook);

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
        .handle_event(worker_started("fixer", &ctx.project_path, &original_hash, "bugs", 1, ""))
        .await
        .unwrap();

    // Verify initial hash
    {
        let workers = ctx.runtime.worker_states.lock();
        assert_eq!(workers["fixer"].runbook_hash, original_hash);
    }

    // Edit the runbook on disk (change the init step command)
    let updated_runbook = runbook.replace("echo init", "echo updated-init");
    let runbook_path = ctx.project_path.join(".oj/runbooks/test.toml");
    std::fs::write(&runbook_path, &updated_runbook).unwrap();

    // Compute the expected new hash
    let (_, expected_new_hash) = hash_runbook(&updated_runbook);
    assert_ne!(original_hash, expected_new_hash, "hashes should differ after edit");

    // Trigger a poll with an empty item list (still triggers refresh)
    let events = ctx
        .runtime
        .handle_event(Event::WorkerPolled {
            worker: "fixer".to_string(),
            project: String::new(),
            items: vec![],
        })
        .await
        .unwrap();

    // The refresh should have emitted a RunbookLoaded event
    let has_runbook_loaded = events.iter().any(|e| matches!(e, Event::RunbookLoaded { .. }));
    assert!(
        has_runbook_loaded,
        "WorkerPolled should emit RunbookLoaded when runbook changed on disk"
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
        assert!(cache.contains_key(&expected_new_hash), "new runbook should be in cache");
    }
}

/// After daemon restart, a worker with concurrency=2 and 2 active jobs
/// should restore both and not dispatch new items.
#[tokio::test]
async fn worker_restart_restores_multiple_active_jobs() {
    let runbook = test_runbook_worker("type = \"persisted\"\nvars = [\"title\"]", 2);
    let ctx = setup_with_runbook(&runbook).await;
    let hash = load_runbook_hash(&ctx, &runbook);

    let ws = worker_started("fixer", &ctx.project_path, &hash, "bugs", 2, "");
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&ws);
        state.apply_event(&Event::WorkerDispatched {
            worker: "fixer".to_string(),
            item_id: "item-1".to_string(),
            owner: JobId::new("job-a").into(),
            project: String::new(),
        });
        state.apply_event(&Event::WorkerDispatched {
            worker: "fixer".to_string(),
            item_id: "item-2".to_string(),
            owner: JobId::new("job-b").into(),
            project: String::new(),
        });
    });

    push_persisted_items(&ctx, "bugs", 1);

    let start_events = ctx
        .runtime
        .handle_event(worker_started("fixer", &ctx.project_path, &hash, "bugs", 2, ""))
        .await
        .unwrap();

    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(state.active.len(), 2, "should restore 2 active jobs from persisted state");
        assert!(state.active.contains(&oj_core::OwnerId::Job(JobId::new("job-a"))));
        assert!(state.active.contains(&oj_core::OwnerId::Job(JobId::new("job-b"))));
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

#[tokio::test]
async fn started_error_worker_not_in_runbook() {
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;
    let hash = load_runbook_hash(&ctx, &runbook);

    let result = ctx
        .runtime
        .handle_event(worker_started("nonexistent", &ctx.project_path, &hash, "bugs", 1, ""))
        .await;

    assert!(result.is_err(), "should error when worker not in runbook");
}

#[tokio::test]
async fn stopped_nonexistent_worker_returns_ok() {
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;

    let result = ctx
        .runtime
        .handle_event(Event::WorkerStopped { worker: "ghost".to_string(), project: String::new() })
        .await;

    assert!(result.is_ok(), "stopping unknown worker should succeed");
}

#[tokio::test]
async fn resized_nonexistent_worker_returns_empty() {
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;

    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker: "ghost".to_string(),
            concurrency: 5,
            project: String::new(),
        })
        .await
        .unwrap();

    assert!(events.is_empty());
}

#[tokio::test]
async fn resized_stopped_worker_returns_empty() {
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;
    let hash = load_runbook_hash(&ctx, &runbook);

    // Start, then stop the worker
    ctx.runtime
        .handle_event(worker_started("fixer", &ctx.project_path, &hash, "bugs", 1, ""))
        .await
        .unwrap();
    ctx.runtime
        .handle_event(Event::WorkerStopped { worker: "fixer".to_string(), project: String::new() })
        .await
        .unwrap();

    let events = ctx
        .runtime
        .handle_event(Event::WorkerResized {
            worker: "fixer".to_string(),
            concurrency: 5,
            project: String::new(),
        })
        .await
        .unwrap();

    assert!(events.is_empty(), "resizing a stopped worker should return empty events");
}

/// When the runbook has not changed on disk, no RunbookLoaded event should be emitted.
#[tokio::test]
async fn worker_no_refresh_when_runbook_unchanged() {
    let runbook = test_runbook_worker("list = \"echo '[]'\"\ntake = \"echo taken\"", 1);
    let ctx = setup_with_runbook(&runbook).await;

    let (runbook_json, hash) = hash_runbook(&runbook);

    ctx.runtime
        .handle_event(Event::RunbookLoaded {
            hash: hash.clone(),
            version: 1,
            runbook: runbook_json,
        })
        .await
        .unwrap();

    ctx.runtime
        .handle_event(worker_started("fixer", &ctx.project_path, &hash, "bugs", 1, ""))
        .await
        .unwrap();

    // Poll with empty items — runbook unchanged on disk
    let events = ctx
        .runtime
        .handle_event(Event::WorkerPolled {
            worker: "fixer".to_string(),
            project: String::new(),
            items: vec![],
        })
        .await
        .unwrap();

    let has_runbook_loaded = events.iter().any(|e| matches!(e, Event::RunbookLoaded { .. }));
    assert!(!has_runbook_loaded, "No RunbookLoaded should be emitted when runbook is unchanged");

    // Hash should remain the same
    {
        let workers = ctx.runtime.worker_states.lock();
        assert_eq!(workers["fixer"].runbook_hash, hash);
    }
}
