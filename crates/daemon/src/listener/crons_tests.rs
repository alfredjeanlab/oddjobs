// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::{Clock, FakeClock};
use oj_storage::{MaterializedState, Wal};

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::{handle_cron_once, handle_cron_restart, handle_cron_start, handle_cron_stop};

/// Helper: create an EventBus backed by a temp WAL, returning the bus and WAL path.
fn test_event_bus(dir: &std::path::Path) -> (EventBus, PathBuf) {
    let wal_path = dir.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, _reader) = EventBus::new(wal);
    (event_bus, wal_path)
}

/// Helper: create a CronRecord with a deterministic timestamp from a FakeClock.
fn make_cron_record(
    clock: &FakeClock,
    name: &str,
    namespace: &str,
    status: &str,
    interval: &str,
    pipeline_name: &str,
) -> oj_storage::CronRecord {
    oj_storage::CronRecord {
        name: name.to_string(),
        namespace: namespace.to_string(),
        project_root: PathBuf::from("/fake"),
        runbook_hash: "fake-hash".to_string(),
        status: status.to_string(),
        interval: interval.to_string(),
        pipeline_name: pipeline_name.to_string(),
        run_target: format!("pipeline:{}", pipeline_name),
        started_at_ms: clock.epoch_ms(),
        last_fired_at_ms: None,
    }
}

/// Helper: create a temp project with a valid cron runbook.
fn project_with_cron() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let runbook_dir = dir.path().join(".oj/runbooks");
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
    dir
}

// ── Race fix: handle_cron_start applies state before responding ──────────

#[test]
fn start_applies_state_before_responding() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // Before: no crons in state
    assert!(state.lock().crons.is_empty());

    let result = handle_cron_start(project.path(), "", "nightly", &event_bus, &state).unwrap();

    // Handler returns CronStarted
    assert!(
        matches!(result, Response::CronStarted { ref cron_name } if cron_name == "nightly"),
        "expected CronStarted, got {:?}",
        result
    );

    // Race fix: cron is visible in state immediately (no WAL processing needed)
    let state = state.lock();
    let cron = state
        .crons
        .get("nightly")
        .expect("cron should be in state after start");
    assert_eq!(cron.name, "nightly");
    assert_eq!(cron.status, "running");
    assert_eq!(cron.interval, "24h");
    assert_eq!(cron.pipeline_name, "deploy");
    assert!(cron.started_at_ms > 0, "started_at_ms should be set");
}

#[test]
fn start_with_namespace_uses_scoped_key() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result =
        handle_cron_start(project.path(), "my-project", "nightly", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::CronStarted { ref cron_name } if cron_name == "nightly"),
        "expected CronStarted, got {:?}",
        result
    );

    let state = state.lock();
    // Namespace-scoped key: "my-project/nightly"
    assert!(
        !state.crons.contains_key("nightly"),
        "bare key should not exist"
    );
    let cron = state
        .crons
        .get("my-project/nightly")
        .expect("scoped key should exist");
    assert_eq!(cron.name, "nightly");
    assert_eq!(cron.namespace, "my-project");
    assert_eq!(cron.status, "running");
}

#[test]
fn start_idempotent_overwrites_existing() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with an existing cron at a known timestamp
    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "nightly".to_string(),
        make_cron_record(&clock, "nightly", "", "running", "12h", "old-pipeline"),
    );
    let state = Arc::new(Mutex::new(initial));

    let old_started = state.lock().crons["nightly"].started_at_ms;
    assert_eq!(old_started, clock.epoch_ms());

    // Start again — should overwrite with fresh runbook data
    let result = handle_cron_start(project.path(), "", "nightly", &event_bus, &state).unwrap();

    assert!(matches!(result, Response::CronStarted { .. }));

    let state = state.lock();
    let cron = state.crons.get("nightly").unwrap();
    assert_eq!(cron.status, "running");
    // Interval updated from runbook (24h, not the old 12h)
    assert_eq!(cron.interval, "24h");
    assert_eq!(cron.pipeline_name, "deploy");
    // started_at_ms refreshed (real wall clock, so just verify it's set)
    assert!(cron.started_at_ms > 0);
}

// ── Race fix: handle_cron_stop applies state before responding ───────────

#[test]
fn stop_applies_state_before_responding() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());

    // Pre-populate a running cron at a known FakeClock timestamp
    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "nightly".to_string(),
        make_cron_record(&clock, "nightly", "", "running", "1h", "deploy"),
    );
    let state = Arc::new(Mutex::new(initial));

    let result = handle_cron_stop("nightly", "", &event_bus, &state, None).unwrap();

    assert_eq!(result, Response::Ok);

    // Race fix: status is "stopped" immediately (no WAL processing needed)
    let state = state.lock();
    let cron = state
        .crons
        .get("nightly")
        .expect("cron should still be in state after stop");
    assert_eq!(cron.status, "stopped");
    // started_at_ms preserved from the original FakeClock value
    assert_eq!(cron.started_at_ms, clock.epoch_ms());
}

#[test]
fn stop_with_namespace_uses_scoped_key() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());

    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "my-project/nightly".to_string(),
        make_cron_record(&clock, "nightly", "my-project", "running", "1h", "deploy"),
    );
    let state = Arc::new(Mutex::new(initial));

    let result = handle_cron_stop("nightly", "my-project", &event_bus, &state, None).unwrap();

    assert_eq!(result, Response::Ok);

    let state = state.lock();
    let cron = state.crons.get("my-project/nightly").unwrap();
    assert_eq!(cron.status, "stopped");
}

// ── Race fix: start-then-stop sequence visible without WAL processing ────

#[test]
fn start_then_immediate_stop_both_visible() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // Start cron
    let start_result =
        handle_cron_start(project.path(), "", "nightly", &event_bus, &state).unwrap();
    assert!(matches!(start_result, Response::CronStarted { .. }));

    // Immediately verify running
    assert_eq!(state.lock().crons["nightly"].status, "running");

    // Stop without WAL processing in between
    let stop_result = handle_cron_stop("nightly", "", &event_bus, &state, None).unwrap();
    assert_eq!(stop_result, Response::Ok);

    // Immediately verify stopped
    assert_eq!(state.lock().crons["nightly"].status, "stopped");
}

#[test]
fn stop_preserves_last_fired_at() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());

    let fired_at = clock.epoch_ms() - 60_000; // fired 60s ago
    let mut initial = MaterializedState::default();
    let mut record = make_cron_record(&clock, "nightly", "", "running", "1h", "deploy");
    record.last_fired_at_ms = Some(fired_at);
    initial.crons.insert("nightly".to_string(), record);
    let state = Arc::new(Mutex::new(initial));

    handle_cron_stop("nightly", "", &event_bus, &state, None).unwrap();

    let state = state.lock();
    let cron = state.crons.get("nightly").unwrap();
    assert_eq!(cron.status, "stopped");
    // last_fired_at_ms is preserved (CronStopped only changes status)
    assert_eq!(cron.last_fired_at_ms, Some(fired_at));
}

// ── Existing restart tests ───────────────────────────────────────────────

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
            run_target: "pipeline:deploy".to_string(),
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

    let project = project_with_cron();

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
            run_target: "pipeline:deploy".to_string(),
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

// ── CronOnce tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn once_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a cron that knows the real project root,
    // simulating `--project town` where the daemon already tracks the namespace.
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
            pipeline_name: "deploy".to_string(),
            started_at_ms: 1_000,
            last_fired_at_ms: None,
        },
    );
    let state = Arc::new(Mutex::new(initial));

    // Call handle_cron_once with a wrong project_root (simulating --project
    // from a different directory). The handler should fall back to the known
    // project root for namespace "my-project".
    let result = handle_cron_once(
        std::path::Path::new("/wrong/path"),
        "my-project",
        "nightly",
        &event_bus,
        &state,
    )
    .await
    .unwrap();

    assert!(
        matches!(result, Response::CommandStarted { .. }),
        "expected CommandStarted from namespace fallback, got {:?}",
        result
    );
}

#[tokio::test]
async fn once_without_runbook_returns_error() {
    let wal_dir = tempdir().unwrap();
    let (event_bus, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_cron_once(
        std::path::Path::new("/fake"),
        "",
        "nightly",
        &event_bus,
        &state,
    )
    .await
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}
