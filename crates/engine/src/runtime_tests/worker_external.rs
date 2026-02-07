// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! External queue dedup, inflight tracking, namespace vars, pending_takes tests

use super::*;

use super::worker::{count_dispatched, load_runbook_hash, start_worker_and_poll};

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
        job.vars.contains_key("var.epic.title"),
        "job.vars should contain var.epic.title (from first declared var, namespaced)"
    );
    assert!(
        job.vars.contains_key("var.epic.labels"),
        "job.vars should contain var.epic.labels (from first declared var, namespaced)"
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

/// A second WorkerStarted event (simulating a race between daemon reconciliation
/// and a concurrent `oj worker start`) should NOT clear inflight_items or
/// pending_takes, which would allow duplicate dispatches for items with in-flight
/// take commands.
///
/// This test reproduces the race condition where:
/// 1. Worker is running, poll finds item A, TakeQueueItem(A) dispatched
/// 2. A duplicate WorkerStarted arrives (reconciliation + listener race)
/// 3. Without the fix: WorkerStarted resets state, inflight_items = {}
/// 4. Next poll sees A again, dispatches duplicate TakeQueueItem(A)
///
/// The fix is defense in depth at two layers:
/// - Daemon listener: emits WorkerWake instead of WorkerStarted for running workers
/// - Engine: handle_worker_started delegates to wake if worker is already Running
#[tokio::test]
async fn duplicate_worker_started_preserves_inflight() {
    let ctx = setup_with_runbook(EXTERNAL_CONCURRENT_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, EXTERNAL_CONCURRENT_RUNBOOK);

    // Start the worker (external queue, concurrency=3)
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 3,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // First poll: dispatch bug-1 (adds to inflight_items, pending_takes=1)
    ctx.runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: vec![serde_json::json!({"id": "bug-1"})],
        })
        .await
        .unwrap();

    // Verify initial state
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(state.pending_takes, 1);
        assert!(state.inflight_items.contains("bug-1"));
    }

    // Send a SECOND WorkerStarted — engine should delegate to wake,
    // preserving inflight_items and pending_takes.
    ctx.runtime
        .handle_event(Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: hash.clone(),
            queue_name: "bugs".to_string(),
            concurrency: 3,
            namespace: String::new(),
        })
        .await
        .unwrap();

    // inflight_items and pending_takes should be preserved
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(
            state.pending_takes, 1,
            "duplicate WorkerStarted should preserve pending_takes"
        );
        assert!(
            state.inflight_items.contains("bug-1"),
            "duplicate WorkerStarted should preserve inflight_items"
        );
    }

    // Second poll with same item — should NOT dispatch duplicate
    ctx.runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items: vec![serde_json::json!({"id": "bug-1"})],
        })
        .await
        .unwrap();

    // pending_takes should still be 1 (no duplicate dispatch)
    {
        let workers = ctx.runtime.worker_states.lock();
        let state = workers.get("fixer").unwrap();
        assert_eq!(
            state.pending_takes, 1,
            "poll after duplicate WorkerStarted should not dispatch duplicate take"
        );
        assert_eq!(
            state.inflight_items.len(),
            1,
            "inflight set should still have exactly 1 item"
        );
    }
}

/// External queue items with numeric IDs (e.g. GitHub issue numbers) should all
/// be dispatched independently. Before the fix, `as_str()` returned None for
/// numeric JSON values, collapsing every item to "unknown" and causing the
/// inflight guard to skip all but the first item.
#[tokio::test]
async fn external_queue_numeric_ids_dispatched_independently() {
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

    // Items with numeric IDs (as returned by e.g. `gh issue list --json number`)
    let items = vec![
        serde_json::json!({"id": 6, "title": "issue six"}),
        serde_json::json!({"id": 7, "title": "issue seven"}),
    ];

    ctx.runtime
        .handle_event(Event::WorkerPollComplete {
            worker_name: "fixer".to_string(),
            items,
        })
        .await
        .unwrap();

    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(
        state.pending_takes, 2,
        "both items with numeric IDs should be dispatched"
    );
    assert!(
        state.inflight_items.contains("6"),
        "numeric id 6 should be in-flight"
    );
    assert!(
        state.inflight_items.contains("7"),
        "numeric id 7 should be in-flight"
    );
}

/// The WORKER_RUNBOOK constant from the parent module (used by pending_takes tests).
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
