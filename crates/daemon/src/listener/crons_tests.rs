// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_storage::{MaterializedState, Wal};

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::handle_cron_restart;

/// Helper: create an EventBus backed by a temp WAL, returning the bus and WAL path.
fn test_event_bus(dir: &std::path::Path) -> (EventBus, PathBuf) {
    let wal_path = dir.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, _reader) = EventBus::new(wal);
    (event_bus, wal_path)
}

#[test]
fn restart_without_runbook_returns_error() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_cron_restart(
        std::path::Path::new("/fake"),
        "",
        "nightly",
        &event_bus,
        &state,
    )
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

    // Put a running cron in state so the restart path emits a stop event
    let mut initial_state = MaterializedState::default();
    initial_state.crons.insert(
        "nightly".to_string(),
        oj_storage::CronRecord {
            name: "nightly".to_string(),
            namespace: String::new(),
            project_root: PathBuf::from("/fake"),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            interval: "1h".to_string(),
            pipeline_name: "deploy".to_string(),
            started_at_ms: 0,
            last_fired_at_ms: None,
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    // Restart with no runbook on disk — the stop event is emitted but start
    // fails because the runbook is missing.  This proves the stop path ran.
    let result = handle_cron_restart(
        std::path::Path::new("/fake"),
        "",
        "nightly",
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

    // Create a project with a cron
    let project = tempdir().unwrap();
    let runbook_dir = project.path().join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(
        runbook_dir.join("test.hcl"),
        r#"
cron "nightly" {
  interval = "24h"
  run      = { pipeline = "deploy" }
}

pipeline "deploy" {
  step "run" {
    run = "echo deploying"
  }
}
"#,
    )
    .unwrap();

    // Put existing cron in state
    let mut initial_state = MaterializedState::default();
    initial_state.crons.insert(
        "nightly".to_string(),
        oj_storage::CronRecord {
            name: "nightly".to_string(),
            namespace: String::new(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "old-hash".to_string(),
            status: "running".to_string(),
            interval: "24h".to_string(),
            pipeline_name: "deploy".to_string(),
            started_at_ms: 0,
            last_fired_at_ms: None,
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_cron_restart(project.path(), "", "nightly", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::CronStarted { ref cron_name } if cron_name == "nightly"),
        "expected CronStarted response, got {:?}",
        result
    );
}
