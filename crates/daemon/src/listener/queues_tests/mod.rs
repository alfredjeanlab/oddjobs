// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

mod drop_and_drain;
mod prune;
mod push;
mod retry;

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::Event;
use oj_storage::{MaterializedState, Wal};

use crate::event_bus::EventBus;

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
queue "tasks" {
  type = "persisted"
  vars = ["task"]
}

worker "processor" {
  source  = { queue = "tasks" }
  handler = { job = "handle" }
}

job "handle" {
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
queue "tasks" {
  type = "persisted"
  vars = ["task"]
}

job "handle" {
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
