// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::Event;
use oj_storage::MaterializedState;

use crate::protocol::Response;

use super::super::handle_queue_push;
use super::{
    drain_events, make_ctx, project_with_queue_and_worker, project_with_queue_only, test_event_bus,
};

/// Helper: create a project dir with an external queue and worker.
fn project_with_external_queue_and_worker() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let runbook_dir = dir.path().join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(
        runbook_dir.join("test.hcl"),
        r#"
queue "issues" {
  type = "external"
  list = "echo '[{\"id\":\"1\",\"title\":\"bug\"}]'"
  take = "echo taking ${item.id}"
}

worker "triager" {
  source  = { queue = "issues" }
  run = { job = "triage" }
}

job "triage" {
  step "run" {
    run = "echo triaging"
  }
}
"#,
    )
    .unwrap();
    dir
}

// ── Push worker management tests ──────────────────────────────────────

#[test]
fn push_auto_starts_stopped_worker() {
    let project = project_with_queue_and_worker();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let data = serde_json::json!({ "task": "test-value" });
    let result = handle_queue_push(&ctx, project.path(), "", "tasks", data).unwrap();

    assert!(
        matches!(result, Response::QueuePushed { ref queue, .. } if queue == "tasks"),
        "expected QueuePushed, got {:?}",
        result
    );

    let events = drain_events(&wal);
    // Expect: QueuePushed, RunbookLoaded, WorkerStarted
    assert_eq!(events.len(), 3, "expected 3 events, got: {:?}", events);

    assert!(
        matches!(&events[0], Event::QueuePushed { queue, .. } if queue == "tasks"),
        "first event should be QueuePushed, got: {:?}",
        events[0]
    );
    assert!(
        matches!(&events[1], Event::RunbookLoaded { .. }),
        "second event should be RunbookLoaded, got: {:?}",
        events[1]
    );
    assert!(
        matches!(
            &events[2],
            Event::WorkerStarted { worker, queue, .. }
            if worker == "processor" && queue == "tasks"
        ),
        "third event should be WorkerStarted for processor, got: {:?}",
        events[2]
    );
}

#[test]
fn push_wakes_running_worker() {
    let project = project_with_queue_and_worker();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a running worker
    let mut initial_state = MaterializedState::default();
    initial_state.workers.insert(
        "processor".to_string(),
        oj_storage::WorkerRecord {
            name: "processor".to_string(),
            project_path: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active: vec![],
            queue: "tasks".to_string(),
            concurrency: 1,
            owners: HashMap::new(),
            project: String::new(),
        },
    );
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    let data = serde_json::json!({ "task": "test-value" });
    let result = handle_queue_push(&ctx, project.path(), "", "tasks", data).unwrap();

    assert!(matches!(result, Response::QueuePushed { .. }));

    let events = drain_events(&wal);
    // Expect: QueuePushed, WorkerWake (not RunbookLoaded + WorkerStarted)
    assert_eq!(events.len(), 2, "expected 2 events, got: {:?}", events);

    assert!(matches!(&events[0], Event::QueuePushed { .. }));
    assert!(
        matches!(
            &events[1],
            Event::WorkerWake { worker, .. } if worker == "processor"
        ),
        "second event should be WorkerWake, got: {:?}",
        events[1]
    );
}

#[test]
fn push_with_no_workers_succeeds() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let data = serde_json::json!({ "task": "test-value" });
    let result = handle_queue_push(&ctx, project.path(), "", "tasks", data).unwrap();

    assert!(matches!(result, Response::QueuePushed { .. }));

    let events = drain_events(&wal);
    // Only QueuePushed, no worker events
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], Event::QueuePushed { .. }));
}

// ── External queue push tests ─────────────────────────────────────────

#[test]
fn push_external_queue_wakes_workers() {
    let project = project_with_external_queue_and_worker();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a running worker
    let mut initial_state = MaterializedState::default();
    initial_state.workers.insert(
        "triager".to_string(),
        oj_storage::WorkerRecord {
            name: "triager".to_string(),
            project_path: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active: vec![],
            queue: "issues".to_string(),
            concurrency: 1,
            owners: HashMap::new(),
            project: String::new(),
        },
    );
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    // Push with empty data — should refresh, not error
    let data = serde_json::json!({});
    let result = handle_queue_push(&ctx, project.path(), "", "issues", data).unwrap();

    assert!(matches!(result, Response::Ok), "expected Ok, got {:?}", result);

    let events = drain_events(&wal);
    // External push should only produce WorkerWake (no QueuePushed event)
    assert_eq!(events.len(), 1, "expected 1 event, got: {:?}", events);
    assert!(
        matches!(
            &events[0],
            Event::WorkerWake { worker, .. } if worker == "triager"
        ),
        "event should be WorkerWake, got: {:?}",
        events[0]
    );
}

#[test]
fn push_external_queue_auto_starts_stopped_worker() {
    let project = project_with_external_queue_and_worker();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let data = serde_json::json!({});
    let result = handle_queue_push(&ctx, project.path(), "", "issues", data).unwrap();

    assert!(matches!(result, Response::Ok), "expected Ok, got {:?}", result);

    let events = drain_events(&wal);
    // Expect: RunbookLoaded, WorkerStarted (auto-start, no QueuePushed)
    assert_eq!(events.len(), 2, "expected 2 events, got: {:?}", events);
    assert!(
        matches!(&events[0], Event::RunbookLoaded { .. }),
        "first event should be RunbookLoaded, got: {:?}",
        events[0]
    );
    assert!(
        matches!(
            &events[1],
            Event::WorkerStarted { worker, queue, .. }
            if worker == "triager" && queue == "issues"
        ),
        "second event should be WorkerStarted, got: {:?}",
        events[1]
    );
}

// ── Push deduplication tests ──────────────────────────────────────────

#[test]
fn push_deduplicates_pending_item_with_same_data() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a pending item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "existing-item-1".to_string(),
        data: [("task".to_string(), "build-feature-x".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    // Push the same data again
    let data = serde_json::json!({ "task": "build-feature-x" });
    let result = handle_queue_push(&ctx, project.path(), "", "tasks", data).unwrap();

    // Should return the existing item ID, not create a new one
    assert!(
        matches!(
            result,
            Response::QueuePushed { ref queue, ref item_id }
            if queue == "tasks" && item_id == "existing-item-1"
        ),
        "expected QueuePushed with existing item ID, got {:?}",
        result
    );

    // No QueuePushed event should have been emitted (dedup)
    let events = drain_events(&wal);
    assert!(
        !events.iter().any(|e| matches!(e, Event::QueuePushed { .. })),
        "no QueuePushed event should be emitted for duplicate, got: {:?}",
        events
    );
}

#[test]
fn push_deduplicates_active_item_with_same_data() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with an active item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "active-item-1".to_string(),
        data: [("task".to_string(), "build-feature-y".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    initial_state.apply_event(&Event::QueueTaken {
        queue: "tasks".to_string(),
        item_id: "active-item-1".to_string(),
        worker: "w1".to_string(),
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    // Push the same data again
    let data = serde_json::json!({ "task": "build-feature-y" });
    let result = handle_queue_push(&ctx, project.path(), "", "tasks", data).unwrap();

    // Should return the existing active item ID
    assert!(
        matches!(
            result,
            Response::QueuePushed { ref queue, ref item_id }
            if queue == "tasks" && item_id == "active-item-1"
        ),
        "expected QueuePushed with existing active item ID, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert!(
        !events.iter().any(|e| matches!(e, Event::QueuePushed { .. })),
        "no QueuePushed event should be emitted for duplicate active item",
    );
}

#[test]
fn push_allows_duplicate_data_when_previous_is_completed() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a completed item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "completed-item-1".to_string(),
        data: [("task".to_string(), "build-feature-z".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    initial_state.apply_event(&Event::QueueCompleted {
        queue: "tasks".to_string(),
        item_id: "completed-item-1".to_string(),
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    // Push the same data again — should succeed since the previous item is completed
    let data = serde_json::json!({ "task": "build-feature-z" });
    let result = handle_queue_push(&ctx, project.path(), "", "tasks", data).unwrap();

    // Should create a new item (different ID from completed one)
    match result {
        Response::QueuePushed { ref queue, ref item_id } => {
            assert_eq!(queue, "tasks");
            assert_ne!(item_id, "completed-item-1", "should be a new item ID");
        }
        other => panic!("expected QueuePushed, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(
        events.iter().any(|e| matches!(e, Event::QueuePushed { .. })),
        "QueuePushed event should be emitted for re-push after completion",
    );
}

#[test]
fn push_allows_duplicate_data_when_previous_is_dead() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a dead item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "dead-item-1".to_string(),
        data: [("task".to_string(), "build-feature-w".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    initial_state.apply_event(&Event::QueueDead {
        queue: "tasks".to_string(),
        item_id: "dead-item-1".to_string(),
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    // Push the same data again — should succeed since the previous item is dead
    let data = serde_json::json!({ "task": "build-feature-w" });
    let result = handle_queue_push(&ctx, project.path(), "", "tasks", data).unwrap();

    match result {
        Response::QueuePushed { ref queue, ref item_id } => {
            assert_eq!(queue, "tasks");
            assert_ne!(item_id, "dead-item-1", "should be a new item ID");
        }
        other => panic!("expected QueuePushed, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(
        events.iter().any(|e| matches!(e, Event::QueuePushed { .. })),
        "QueuePushed event should be emitted for re-push after dead",
    );
}

#[test]
fn push_different_data_is_not_deduplicated() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a pending item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "existing-item-1".to_string(),
        data: [("task".to_string(), "build-feature-x".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    // Push different data — should create a new item
    let data = serde_json::json!({ "task": "build-feature-y" });
    let result = handle_queue_push(&ctx, project.path(), "", "tasks", data).unwrap();

    match result {
        Response::QueuePushed { ref queue, ref item_id } => {
            assert_eq!(queue, "tasks");
            assert_ne!(item_id, "existing-item-1", "should be a new item ID");
        }
        other => panic!("expected QueuePushed, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(
        events.iter().any(|e| matches!(e, Event::QueuePushed { .. })),
        "QueuePushed event should be emitted for different data",
    );
}

// ── Push project fallback test ──────────────────────────────────────

#[test]
fn push_with_wrong_project_path_falls_back_to_namespace() {
    let project = project_with_queue_and_worker();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a worker that knows the real project root,
    // simulating `--project my-project` where the daemon already tracks the project.
    let mut initial = MaterializedState::default();
    initial.workers.insert(
        "my-project/processor".to_string(),
        oj_storage::WorkerRecord {
            name: "processor".to_string(),
            project_path: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active: vec![],
            queue: "tasks".to_string(),
            concurrency: 1,
            owners: HashMap::new(),
            project: "my-project".to_string(),
        },
    );
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial)));

    // Call with a wrong project_path (simulating --project from a different directory).
    let data = serde_json::json!({ "task": "test-value" });
    let result =
        handle_queue_push(&ctx, std::path::Path::new("/wrong/path"), "my-project", "tasks", data)
            .unwrap();

    assert!(
        matches!(result, Response::QueuePushed { ref queue, .. } if queue == "tasks"),
        "expected QueuePushed from project fallback, got {:?}",
        result
    );
}
