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
