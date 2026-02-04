// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::Event;
use oj_storage::{MaterializedState, Wal};

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::{
    handle_queue_drain, handle_queue_drop, handle_queue_prune, handle_queue_push,
    handle_queue_retry,
};

/// Helper: create an EventBus backed by a temp WAL, returning the bus, reader WAL arc, and path.
fn test_event_bus(dir: &std::path::Path) -> (EventBus, Arc<Mutex<Wal>>, PathBuf) {
    let wal_path = dir.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, reader) = EventBus::new(wal);
    let wal = reader.wal();
    (event_bus, wal, wal_path)
}

/// Helper: create a project dir with a runbook containing a persisted queue and worker.
fn project_with_queue_and_worker() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let runbook_dir = dir.path().join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(
        runbook_dir.join("test.hcl"),
        r#"
queue "jobs" {
  type = "persisted"
  vars = ["task"]
}

worker "processor" {
  source  = { queue = "jobs" }
  handler = { pipeline = "handle" }
}

pipeline "handle" {
  step "run" {
    run = "echo task"
  }
}
"#,
    )
    .unwrap();
    dir
}

/// Helper: create a project dir with a persisted queue but no worker.
fn project_with_queue_only() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let runbook_dir = dir.path().join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(
        runbook_dir.join("test.hcl"),
        r#"
queue "jobs" {
  type = "persisted"
  vars = ["task"]
}

pipeline "handle" {
  step "run" {
    run = "echo task"
  }
}
"#,
    )
    .unwrap();
    dir
}

/// Collect all events from the WAL.
fn drain_events(wal: &Arc<Mutex<Wal>>) -> Vec<Event> {
    let mut events = Vec::new();
    let mut wal = wal.lock();
    while let Some(entry) = wal.next_unprocessed().unwrap() {
        events.push(entry.event);
        wal.mark_processed(entry.seq);
    }
    events
}

#[test]
fn push_auto_starts_stopped_worker() {
    let project = project_with_queue_and_worker();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let data = serde_json::json!({ "task": "test-value" });
    let result = handle_queue_push(project.path(), "", "jobs", data, &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::QueuePushed { ref queue_name, .. } if queue_name == "jobs"),
        "expected QueuePushed, got {:?}",
        result
    );

    let events = drain_events(&wal);
    // Expect: QueuePushed, RunbookLoaded, WorkerStarted
    assert_eq!(events.len(), 3, "expected 3 events, got: {:?}", events);

    assert!(
        matches!(&events[0], Event::QueuePushed { queue_name, .. } if queue_name == "jobs"),
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
            Event::WorkerStarted { worker_name, queue_name, .. }
            if worker_name == "processor" && queue_name == "jobs"
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
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active_pipeline_ids: vec![],
            queue_name: "jobs".to_string(),
            concurrency: 1,
            namespace: String::new(),
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    let data = serde_json::json!({ "task": "test-value" });
    let result = handle_queue_push(project.path(), "", "jobs", data, &event_bus, &state).unwrap();

    assert!(matches!(result, Response::QueuePushed { .. }));

    let events = drain_events(&wal);
    // Expect: QueuePushed, WorkerWake (not RunbookLoaded + WorkerStarted)
    assert_eq!(events.len(), 2, "expected 2 events, got: {:?}", events);

    assert!(matches!(&events[0], Event::QueuePushed { .. }));
    assert!(
        matches!(
            &events[1],
            Event::WorkerWake { worker_name, .. } if worker_name == "processor"
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

    let data = serde_json::json!({ "task": "test-value" });
    let result = handle_queue_push(project.path(), "", "jobs", data, &event_bus, &state).unwrap();

    assert!(matches!(result, Response::QueuePushed { .. }));

    let events = drain_events(&wal);
    // Only QueuePushed, no worker events
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], Event::QueuePushed { .. }));
}

#[test]
fn drop_removes_item_from_queue() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a pushed item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "item-abc123".to_string(),
        data: [("task".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_queue_drop(
        project.path(),
        "",
        "jobs",
        "item-abc123",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref queue_name, ref item_id }
            if queue_name == "jobs" && item_id == "item-abc123"
        ),
        "expected QueueDropped, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped {
            queue_name,
            item_id,
            ..
        } if queue_name == "jobs" && item_id == "item-abc123"
    ));
}

#[test]
fn drop_unknown_queue_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_drop(
        project.path(),
        "",
        "nonexistent",
        "item-1",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("nonexistent")),
        "expected Error, got {:?}",
        result
    );
}

#[test]
fn drop_nonexistent_item_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_drop(
        project.path(),
        "",
        "jobs",
        "item-missing",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("not found")),
        "expected Error about item not found, got {:?}",
        result
    );
}

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
  handler = { pipeline = "triage" }
}

pipeline "triage" {
  step "run" {
    run = "echo triaging"
  }
}
"#,
    )
    .unwrap();
    dir
}

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
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active_pipeline_ids: vec![],
            queue_name: "issues".to_string(),
            concurrency: 1,
            namespace: String::new(),
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    // Push with empty data — should refresh, not error
    let data = serde_json::json!({});
    let result = handle_queue_push(project.path(), "", "issues", data, &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Ok),
        "expected Ok, got {:?}",
        result
    );

    let events = drain_events(&wal);
    // External push should only produce WorkerWake (no QueuePushed event)
    assert_eq!(events.len(), 1, "expected 1 event, got: {:?}", events);
    assert!(
        matches!(
            &events[0],
            Event::WorkerWake { worker_name, .. } if worker_name == "triager"
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

    let data = serde_json::json!({});
    let result = handle_queue_push(project.path(), "", "issues", data, &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Ok),
        "expected Ok, got {:?}",
        result
    );

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
            Event::WorkerStarted { worker_name, queue_name, .. }
            if worker_name == "triager" && queue_name == "issues"
        ),
        "second event should be WorkerStarted, got: {:?}",
        events[1]
    );
}

/// Helper: push an item and mark it as Dead so it can be retried.
fn push_and_mark_dead(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    data: &[(&str, &str)],
) {
    let data_map = data
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: data_map,
        pushed_at_epoch_ms: 1_000_000,
        namespace: namespace.to_string(),
    });
    state.lock().apply_event(&Event::QueueItemDead {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: namespace.to_string(),
    });
}

#[test]
fn drop_with_prefix_resolves_unique_match() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "abc12345-0000-0000-0000-000000000000".to_string(),
        data: [("task".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    let result =
        handle_queue_drop(project.path(), "", "jobs", "abc12", &event_bus, &state).unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref item_id, .. }
            if item_id == "abc12345-0000-0000-0000-000000000000"
        ),
        "expected QueueDropped with full ID, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. }
        if item_id == "abc12345-0000-0000-0000-000000000000"
    ));
}

#[test]
fn drop_ambiguous_prefix_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    let mut initial_state = MaterializedState::default();
    for suffix in ["aaa", "bbb"] {
        initial_state.apply_event(&Event::QueuePushed {
            queue_name: "jobs".to_string(),
            item_id: format!("abc-{}", suffix),
            data: [("task".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            pushed_at_epoch_ms: 1_000_000,
            namespace: String::new(),
        });
    }
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_queue_drop(project.path(), "", "jobs", "abc", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("ambiguous")),
        "expected ambiguous error, got {:?}",
        result
    );
}

#[test]
fn retry_with_prefix_resolves_unique_match() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_dead(
        &state,
        "",
        "jobs",
        "def98765-0000-0000-0000-000000000000",
        &[("task", "retry-me")],
    );

    let result =
        handle_queue_retry(project.path(), "", "jobs", "def98", &event_bus, &state).unwrap();

    assert!(
        matches!(
            result,
            Response::QueueRetried { ref item_id, .. }
            if item_id == "def98765-0000-0000-0000-000000000000"
        ),
        "expected QueueRetried with full ID, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::QueueItemRetry { item_id, .. } if item_id == "def98765-0000-0000-0000-000000000000")));
}

#[test]
fn retry_with_exact_id_still_works() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_dead(&state, "", "jobs", "exact-id-1234", &[("task", "retry-me")]);

    let result = handle_queue_retry(
        project.path(),
        "",
        "jobs",
        "exact-id-1234",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueRetried { ref item_id, .. }
            if item_id == "exact-id-1234"
        ),
        "expected QueueRetried, got {:?}",
        result
    );
}

#[test]
fn retry_ambiguous_prefix_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    for suffix in ["aaa", "bbb"] {
        push_and_mark_dead(
            &state,
            "",
            "jobs",
            &format!("abc-{}", suffix),
            &[("task", "test")],
        );
    }

    let result = handle_queue_retry(project.path(), "", "jobs", "abc", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("ambiguous")),
        "expected ambiguous error, got {:?}",
        result
    );
}

#[test]
fn retry_no_match_returns_not_found() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_retry(
        project.path(),
        "",
        "jobs",
        "nonexistent",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("not found")),
        "expected not found error, got {:?}",
        result
    );
}

#[test]
fn drain_removes_all_pending_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with multiple pending items
    let mut initial_state = MaterializedState::default();
    for i in 1..=3 {
        initial_state.apply_event(&Event::QueuePushed {
            queue_name: "jobs".to_string(),
            item_id: format!("item-{}", i),
            data: [("task".to_string(), format!("task-{}", i))]
                .into_iter()
                .collect(),
            pushed_at_epoch_ms: 1_000_000 + i,
            namespace: String::new(),
        });
    }
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_queue_drain(project.path(), "", "jobs", &event_bus, &state).unwrap();

    match result {
        Response::QueueDrained {
            ref queue_name,
            ref items,
        } => {
            assert_eq!(queue_name, "jobs");
            assert_eq!(items.len(), 3);
            let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
            assert!(ids.contains(&"item-1"));
            assert!(ids.contains(&"item-2"));
            assert!(ids.contains(&"item-3"));
        }
        other => panic!("expected QueueDrained, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 3, "expected 3 QueueDropped events");
    for event in &events {
        assert!(
            matches!(event, Event::QueueDropped { queue_name, .. } if queue_name == "jobs"),
            "expected QueueDropped, got {:?}",
            event
        );
    }
}

#[test]
fn drain_skips_non_pending_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    let mut initial_state = MaterializedState::default();
    // One pending item
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "pending".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    // One active item
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "active-1".to_string(),
        data: [("task".to_string(), "active".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 2_000_000,
        namespace: String::new(),
    });
    initial_state.apply_event(&Event::QueueTaken {
        queue_name: "jobs".to_string(),
        item_id: "active-1".to_string(),
        worker_name: "w1".to_string(),
        namespace: String::new(),
    });
    // One dead item
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "dead-1".to_string(),
        data: [("task".to_string(), "dead".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 3_000_000,
        namespace: String::new(),
    });
    initial_state.apply_event(&Event::QueueItemDead {
        queue_name: "jobs".to_string(),
        item_id: "dead-1".to_string(),
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_queue_drain(project.path(), "", "jobs", &event_bus, &state).unwrap();

    match result {
        Response::QueueDrained { ref items, .. } => {
            assert_eq!(items.len(), 1, "only pending items should be drained");
            assert_eq!(items[0].id, "pending-1");
        }
        other => panic!("expected QueueDrained, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. } if item_id == "pending-1"
    ));
}

#[test]
fn drain_empty_queue_returns_empty_list() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_drain(project.path(), "", "jobs", &event_bus, &state).unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDrained { ref queue_name, ref items }
            if queue_name == "jobs" && items.is_empty()
        ),
        "expected empty QueueDrained, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert!(
        events.is_empty(),
        "no events should be emitted for empty drain"
    );
}

#[test]
fn push_deduplicates_pending_item_with_same_data() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a pending item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "existing-item-1".to_string(),
        data: [("task".to_string(), "build-feature-x".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    // Push the same data again
    let data = serde_json::json!({ "task": "build-feature-x" });
    let result = handle_queue_push(project.path(), "", "jobs", data, &event_bus, &state).unwrap();

    // Should return the existing item ID, not create a new one
    assert!(
        matches!(
            result,
            Response::QueuePushed { ref queue_name, ref item_id }
            if queue_name == "jobs" && item_id == "existing-item-1"
        ),
        "expected QueuePushed with existing item ID, got {:?}",
        result
    );

    // No QueuePushed event should have been emitted (dedup)
    let events = drain_events(&wal);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::QueuePushed { .. })),
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
        queue_name: "jobs".to_string(),
        item_id: "active-item-1".to_string(),
        data: [("task".to_string(), "build-feature-y".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    initial_state.apply_event(&Event::QueueTaken {
        queue_name: "jobs".to_string(),
        item_id: "active-item-1".to_string(),
        worker_name: "w1".to_string(),
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    // Push the same data again
    let data = serde_json::json!({ "task": "build-feature-y" });
    let result = handle_queue_push(project.path(), "", "jobs", data, &event_bus, &state).unwrap();

    // Should return the existing active item ID
    assert!(
        matches!(
            result,
            Response::QueuePushed { ref queue_name, ref item_id }
            if queue_name == "jobs" && item_id == "active-item-1"
        ),
        "expected QueuePushed with existing active item ID, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::QueuePushed { .. })),
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
        queue_name: "jobs".to_string(),
        item_id: "completed-item-1".to_string(),
        data: [("task".to_string(), "build-feature-z".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    initial_state.apply_event(&Event::QueueCompleted {
        queue_name: "jobs".to_string(),
        item_id: "completed-item-1".to_string(),
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    // Push the same data again — should succeed since the previous item is completed
    let data = serde_json::json!({ "task": "build-feature-z" });
    let result = handle_queue_push(project.path(), "", "jobs", data, &event_bus, &state).unwrap();

    // Should create a new item (different ID from completed one)
    match result {
        Response::QueuePushed {
            ref queue_name,
            ref item_id,
        } => {
            assert_eq!(queue_name, "jobs");
            assert_ne!(item_id, "completed-item-1", "should be a new item ID");
        }
        other => panic!("expected QueuePushed, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::QueuePushed { .. })),
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
        queue_name: "jobs".to_string(),
        item_id: "dead-item-1".to_string(),
        data: [("task".to_string(), "build-feature-w".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    initial_state.apply_event(&Event::QueueItemDead {
        queue_name: "jobs".to_string(),
        item_id: "dead-item-1".to_string(),
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    // Push the same data again — should succeed since the previous item is dead
    let data = serde_json::json!({ "task": "build-feature-w" });
    let result = handle_queue_push(project.path(), "", "jobs", data, &event_bus, &state).unwrap();

    match result {
        Response::QueuePushed {
            ref queue_name,
            ref item_id,
        } => {
            assert_eq!(queue_name, "jobs");
            assert_ne!(item_id, "dead-item-1", "should be a new item ID");
        }
        other => panic!("expected QueuePushed, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::QueuePushed { .. })),
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
        queue_name: "jobs".to_string(),
        item_id: "existing-item-1".to_string(),
        data: [("task".to_string(), "build-feature-x".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    // Push different data — should create a new item
    let data = serde_json::json!({ "task": "build-feature-y" });
    let result = handle_queue_push(project.path(), "", "jobs", data, &event_bus, &state).unwrap();

    match result {
        Response::QueuePushed {
            ref queue_name,
            ref item_id,
        } => {
            assert_eq!(queue_name, "jobs");
            assert_ne!(item_id, "existing-item-1", "should be a new item ID");
        }
        other => panic!("expected QueuePushed, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::QueuePushed { .. })),
        "QueuePushed event should be emitted for different data",
    );
}

#[test]
fn drain_unknown_queue_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_drain(project.path(), "", "nonexistent", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("nonexistent")),
        "expected Error, got {:?}",
        result
    );
}

// ── --project flag: namespace fallback tests ─────────────────────────────

#[test]
fn push_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_queue_and_worker();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a worker that knows the real project root,
    // simulating `--project my-project` where the daemon already tracks the namespace.
    let mut initial = MaterializedState::default();
    initial.workers.insert(
        "my-project/processor".to_string(),
        oj_storage::WorkerRecord {
            name: "processor".to_string(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active_pipeline_ids: vec![],
            queue_name: "jobs".to_string(),
            concurrency: 1,
            namespace: "my-project".to_string(),
        },
    );
    let state = Arc::new(Mutex::new(initial));

    // Call with a wrong project_root (simulating --project from a different directory).
    let data = serde_json::json!({ "task": "test-value" });
    let result = handle_queue_push(
        std::path::Path::new("/wrong/path"),
        "my-project",
        "jobs",
        data,
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::QueuePushed { ref queue_name, .. } if queue_name == "jobs"),
        "expected QueuePushed from namespace fallback, got {:?}",
        result
    );
}

#[test]
fn drop_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a cron that knows the real project root
    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "my-project/nightly".to_string(),
        oj_storage::CronRecord {
            name: "nightly".to_string(),
            namespace: "my-project".to_string(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            interval: "24h".to_string(),
            pipeline_name: "handle".to_string(),
            run_target: "pipeline:handle".to_string(),
            started_at_ms: 1_000,
            last_fired_at_ms: None,
        },
    );
    // Also add a queue item so the drop has something to find
    initial.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "item-abc123".to_string(),
        data: [("task".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: "my-project".to_string(),
    });
    let state = Arc::new(Mutex::new(initial));

    let result = handle_queue_drop(
        std::path::Path::new("/wrong/path"),
        "my-project",
        "jobs",
        "item-abc123",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref queue_name, ref item_id }
            if queue_name == "jobs" && item_id == "item-abc123"
        ),
        "expected QueueDropped from namespace fallback, got {:?}",
        result
    );
}

#[test]
fn retry_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a worker that knows the real project root
    let mut initial = MaterializedState::default();
    initial.workers.insert(
        "my-project/processor".to_string(),
        oj_storage::WorkerRecord {
            name: "processor".to_string(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "stopped".to_string(),
            active_pipeline_ids: vec![],
            queue_name: "jobs".to_string(),
            concurrency: 1,
            namespace: "my-project".to_string(),
        },
    );
    // Add a dead queue item to retry
    push_and_mark_dead(
        &Arc::new(Mutex::new(MaterializedState::default())),
        "my-project",
        "jobs",
        "item-dead-1",
        &[("task", "retry-me")],
    );
    // Apply directly to initial state
    initial.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "item-dead-1".to_string(),
        data: [("task".to_string(), "retry-me".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: "my-project".to_string(),
    });
    initial.apply_event(&Event::QueueItemDead {
        queue_name: "jobs".to_string(),
        item_id: "item-dead-1".to_string(),
        namespace: "my-project".to_string(),
    });
    let state = Arc::new(Mutex::new(initial));

    let result = handle_queue_retry(
        std::path::Path::new("/wrong/path"),
        "my-project",
        "jobs",
        "item-dead-1",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueRetried { ref queue_name, ref item_id }
            if queue_name == "jobs" && item_id == "item-dead-1"
        ),
        "expected QueueRetried from namespace fallback, got {:?}",
        result
    );
}

#[test]
fn drain_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a cron that knows the real project root
    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "my-project/nightly".to_string(),
        oj_storage::CronRecord {
            name: "nightly".to_string(),
            namespace: "my-project".to_string(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            interval: "24h".to_string(),
            pipeline_name: "handle".to_string(),
            run_target: "pipeline:handle".to_string(),
            started_at_ms: 1_000,
            last_fired_at_ms: None,
        },
    );
    // Add pending queue items
    initial.apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: "my-project".to_string(),
    });
    let state = Arc::new(Mutex::new(initial));

    let result = handle_queue_drain(
        std::path::Path::new("/wrong/path"),
        "my-project",
        "jobs",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDrained { ref queue_name, ref items }
            if queue_name == "jobs" && items.len() == 1
        ),
        "expected QueueDrained from namespace fallback, got {:?}",
        result
    );
}

// ── Queue prune tests ─────────────────────────────────────────────────

/// Helper: push an item and mark it as Completed.
fn push_and_mark_completed(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    data: &[(&str, &str)],
    pushed_at_epoch_ms: u64,
) {
    let data_map = data
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: data_map,
        pushed_at_epoch_ms,
        namespace: namespace.to_string(),
    });
    state.lock().apply_event(&Event::QueueCompleted {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: namespace.to_string(),
    });
}

/// Helper: push an item and mark it as Dead, with a specific pushed_at timestamp.
fn push_and_mark_dead_at(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    data: &[(&str, &str)],
    pushed_at_epoch_ms: u64,
) {
    let data_map = data
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: data_map,
        pushed_at_epoch_ms,
        namespace: namespace.to_string(),
    });
    state.lock().apply_event(&Event::QueueItemDead {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: namespace.to_string(),
    });
}

/// Helper: push an item and mark it as Failed.
fn push_and_mark_failed(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    data: &[(&str, &str)],
    pushed_at_epoch_ms: u64,
) {
    let data_map = data
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: data_map,
        pushed_at_epoch_ms,
        namespace: namespace.to_string(),
    });
    state.lock().apply_event(&Event::QueueFailed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        error: "test error".to_string(),
        namespace: namespace.to_string(),
    });
}

/// Old timestamp: 24 hours ago (well past the 12h threshold).
fn old_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 24 * 60 * 60 * 1000
}

/// Recent timestamp: 1 hour ago (within the 12h threshold).
fn recent_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 1 * 60 * 60 * 1000
}

#[test]
fn prune_completed_items_older_than_12h() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_completed(
        &state,
        "",
        "jobs",
        "old-item-1",
        &[("task", "a")],
        old_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "jobs", false, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].item_id, "old-item-1");
            assert_eq!(pruned[0].status, "completed");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. } if item_id == "old-item-1"
    ));
}

#[test]
fn prune_skips_recent_completed_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_completed(
        &state,
        "",
        "jobs",
        "recent-item-1",
        &[("task", "b")],
        recent_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "jobs", false, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert!(pruned.is_empty(), "recent items should not be pruned");
            assert_eq!(skipped, 1);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty(), "no events should be emitted");
}

#[test]
fn prune_all_flag_prunes_recent_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_completed(
        &state,
        "",
        "jobs",
        "recent-item-1",
        &[("task", "c")],
        recent_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "jobs", true, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].item_id, "recent-item-1");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
}

#[test]
fn prune_dead_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_dead_at(
        &state,
        "",
        "jobs",
        "dead-item-1",
        &[("task", "d")],
        old_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "jobs", false, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].item_id, "dead-item-1");
            assert_eq!(pruned[0].status, "dead");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. } if item_id == "dead-item-1"
    ));
}

#[test]
fn prune_skips_active_pending_failed_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // Pending item
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "p".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: old_epoch_ms(),
        namespace: String::new(),
    });

    // Active item
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: "jobs".to_string(),
        item_id: "active-1".to_string(),
        data: [("task".to_string(), "a".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: old_epoch_ms(),
        namespace: String::new(),
    });
    state.lock().apply_event(&Event::QueueTaken {
        queue_name: "jobs".to_string(),
        item_id: "active-1".to_string(),
        worker_name: "w1".to_string(),
        namespace: String::new(),
    });

    // Failed item
    push_and_mark_failed(
        &state,
        "",
        "jobs",
        "failed-1",
        &[("task", "f")],
        old_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "jobs", true, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert!(pruned.is_empty(), "no terminal items to prune");
            assert_eq!(skipped, 3, "pending, active, and failed should be skipped");
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty(), "no events should be emitted");
}

#[test]
fn prune_dry_run_does_not_emit_events() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_completed(
        &state,
        "",
        "jobs",
        "old-item-1",
        &[("task", "a")],
        old_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "jobs", false, true, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert_eq!(pruned.len(), 1, "should report items that would be pruned");
            assert_eq!(pruned[0].item_id, "old-item-1");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(
        events.is_empty(),
        "dry-run should not emit any events, got {:?}",
        events
    );
}

#[test]
fn prune_empty_queue_returns_empty() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result =
        handle_queue_prune(project.path(), "", "jobs", true, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert!(pruned.is_empty());
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty());
}
