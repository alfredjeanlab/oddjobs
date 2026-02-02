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

use super::handle_queue_push;

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
