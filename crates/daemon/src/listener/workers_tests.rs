// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_storage::{MaterializedState, Wal};

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::{handle_worker_restart, handle_worker_start, handle_worker_stop};

/// Helper: create an EventBus backed by a temp WAL, returning the bus and WAL path.
fn test_event_bus(dir: &std::path::Path) -> (EventBus, PathBuf) {
    let wal_path = dir.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, _reader) = EventBus::new(wal);
    (event_bus, wal_path)
}

#[test]
fn start_does_full_start_even_after_restart() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // No runbook on disk, so start should fail with runbook-not-found.
    // This proves it always does a full start (loads runbook) regardless
    // of any stale WAL state.
    let result =
        handle_worker_start(std::path::Path::new("/fake"), "", "fix", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}

#[test]
fn start_suggests_similar_worker_name() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // Create a project with a worker named "processor"
    let project = tempdir().unwrap();
    let runbook_dir = project.path().join(".oj/runbooks");
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

    let result = handle_worker_start(project.path(), "", "processer", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("did you mean: processor?")),
        "expected suggestion for 'processor', got {:?}",
        result
    );
}

#[test]
fn stop_unknown_worker_returns_error() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_worker_stop("nonexistent", "", &event_bus, &state, None).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("unknown worker")),
        "expected unknown worker error, got {:?}",
        result
    );
}

#[test]
fn stop_suggests_similar_worker_from_state() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());

    let mut initial_state = MaterializedState::default();
    initial_state.workers.insert(
        "processor".to_string(),
        oj_storage::WorkerRecord {
            name: "processor".to_string(),
            project_root: PathBuf::from("/fake"),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active_job_ids: vec![],
            queue_name: "tasks".to_string(),
            concurrency: 1,
            namespace: String::new(),
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_worker_stop("processer", "", &event_bus, &state, None).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("did you mean: processor?")),
        "expected suggestion for 'processor', got {:?}",
        result
    );
}

#[test]
fn stop_suggests_cross_namespace_worker() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());

    let mut initial_state = MaterializedState::default();
    initial_state.workers.insert(
        "other-project/fix".to_string(),
        oj_storage::WorkerRecord {
            name: "fix".to_string(),
            project_root: PathBuf::from("/other"),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active_job_ids: vec![],
            queue_name: "issues".to_string(),
            concurrency: 1,
            namespace: "other-project".to_string(),
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_worker_stop("fix", "my-project", &event_bus, &state, None).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("--project other-project")),
        "expected cross-project suggestion, got {:?}",
        result
    );
}

#[test]
fn restart_without_runbook_returns_error() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result =
        handle_worker_restart(std::path::Path::new("/fake"), "", "fix", &event_bus, &state)
            .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}

#[test]
fn restart_stops_existing_then_starts() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());

    // Put a running worker in state so the restart path emits a stop event
    let mut initial_state = MaterializedState::default();
    initial_state.workers.insert(
        "processor".to_string(),
        oj_storage::WorkerRecord {
            name: "processor".to_string(),
            project_root: PathBuf::from("/fake"),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            active_job_ids: vec![],
            queue_name: "tasks".to_string(),
            concurrency: 1,
            namespace: String::new(),
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    // Restart with no runbook on disk — the stop event is emitted but start
    // fails because the runbook is missing.  This proves the stop path ran.
    let result = handle_worker_restart(
        std::path::Path::new("/fake"),
        "",
        "processor",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error after stop, got {:?}",
        result
    );
}

#[test]
fn restart_with_valid_runbook_returns_started() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());

    // Create a project with a worker
    let project = tempdir().unwrap();
    let runbook_dir = project.path().join(".oj/runbooks");
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

    // Put existing worker in state
    let mut initial_state = MaterializedState::default();
    initial_state.workers.insert(
        "processor".to_string(),
        oj_storage::WorkerRecord {
            name: "processor".to_string(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "old-hash".to_string(),
            status: "running".to_string(),
            active_job_ids: vec![],
            queue_name: "tasks".to_string(),
            concurrency: 1,
            namespace: String::new(),
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    let result =
        handle_worker_restart(project.path(), "", "processor", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::WorkerStarted { ref worker_name } if worker_name == "processor"),
        "expected WorkerStarted response, got {:?}",
        result
    );
}
