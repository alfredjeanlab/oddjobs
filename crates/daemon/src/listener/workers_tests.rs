// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use crate::storage::{MaterializedState, Wal};

use crate::protocol::Response;

use super::super::{resolve_effective_project_path, test_ctx};
use super::{handle_worker_restart, handle_worker_start, handle_worker_stop};

/// Collect all events from the WAL.
fn drain_events(wal: &Arc<Mutex<Wal>>) -> Vec<oj_core::Event> {
    let mut events = Vec::new();
    let mut wal = wal.lock();
    while let Some(entry) = wal.next_unprocessed().unwrap() {
        events.push(entry.event);
        wal.mark_processed(entry.seq);
    }
    events
}

#[test]
fn start_does_full_start_even_after_restart() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

    // No runbook on disk, so start should fail with runbook-not-found.
    // This proves it always does a full start (loads runbook) regardless
    // of any stale WAL state.
    let result =
        handle_worker_start(&ctx, std::path::Path::new("/fake"), "", "fix", false).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}

#[test]
fn start_suggests_similar_worker_name() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

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
  run = { job = "handle" }
}

job "handle" {
  step "run" {
    run = "echo task"
  }
}
"#,
    )
    .unwrap();

    let result = handle_worker_start(&ctx, project.path(), "", "processer", false).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("did you mean: processor?")),
        "expected suggestion for 'processor', got {:?}",
        result
    );
}

#[yare::parameterized(
    unknown_worker  = { None,                                                     "nonexistent", "",           "unknown worker" },
    similar_name    = { Some(("processor", "",              "processor")),          "processer",   "",           "did you mean: processor?" },
    cross_namespace = { Some(("fix",       "other-project", "other-project/fix")), "fix",         "my-project", "--project other-project" },
)]
fn stop_error_suggestions(
    worker: Option<(&str, &str, &str)>,
    query_name: &str,
    query_ns: &str,
    expected_msg: &str,
) {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    if let Some((name, ns, key)) = worker {
        let mut state = ctx.state.lock();
        state.workers.insert(
            key.to_string(),
            crate::storage::WorkerRecord {
                name: name.to_string(),
                project_path: PathBuf::from("/fake"),
                runbook_hash: "fake-hash".to_string(),
                status: "running".to_string(),
                active: vec![],
                queue: "tasks".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
                project: ns.to_string(),
            },
        );
    }

    let result = handle_worker_stop(&ctx, query_name, query_ns, None, false).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains(expected_msg)),
        "expected '{}' in error, got {:?}",
        expected_msg,
        result
    );
}

#[test]
fn restart_without_runbook_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

    let result = handle_worker_restart(&ctx, std::path::Path::new("/fake"), "", "fix").unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}

#[test]
fn restart_stops_existing_then_starts() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

    // Put a running worker in state so the restart path emits a stop event
    {
        let mut state = ctx.state.lock();
        state.workers.insert(
            "processor".to_string(),
            crate::storage::WorkerRecord {
                name: "processor".to_string(),
                project_path: PathBuf::from("/fake"),
                runbook_hash: "fake-hash".to_string(),
                status: "running".to_string(),
                active: vec![],
                queue: "tasks".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
                project: String::new(),
            },
        );
    }

    // Restart with no runbook on disk — the stop event is emitted but start
    // fails because the runbook is missing.  This proves the stop path ran.
    let result =
        handle_worker_restart(&ctx, std::path::Path::new("/fake"), "", "processor").unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error after stop, got {:?}",
        result
    );
}

#[test]
fn restart_with_valid_runbook_returns_started() {
    let dir = tempdir().unwrap();
    let (ctx, wal) = super::super::test_ctx_with_wal(dir.path());

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
  run = { job = "handle" }
}

job "handle" {
  step "run" {
    run = "echo task"
  }
}
"#,
    )
    .unwrap();

    // Put existing worker in state as "running"
    {
        let mut state = ctx.state.lock();
        state.workers.insert(
            "processor".to_string(),
            crate::storage::WorkerRecord {
                name: "processor".to_string(),
                project_path: project.path().to_path_buf(),
                runbook_hash: "old-hash".to_string(),
                status: "running".to_string(),
                active: vec![],
                queue: "tasks".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
                project: String::new(),
            },
        );
    }

    let result = handle_worker_restart(&ctx, project.path(), "", "processor").unwrap();

    assert!(
        matches!(result, Response::WorkerStarted { ref worker } if worker == "processor"),
        "expected WorkerStarted response, got {:?}",
        result
    );

    // Verify materialized state shows "running" (not stuck on "stopped")
    let status = ctx.state.lock().workers["processor"].status.clone();
    assert_eq!(
        status, "running",
        "worker status should be 'running' after restart, got '{}'",
        status
    );

    // Verify emitted events: WorkerStopped then WorkerStarted (not WorkerWake)
    let events = drain_events(&wal);
    assert!(
        events.iter().any(|e| matches!(e, oj_core::Event::WorkerStopped { .. })),
        "restart should emit WorkerStopped"
    );
    assert!(
        events.iter().any(|e| matches!(e, oj_core::Event::WorkerStarted { .. })),
        "restart should emit WorkerStarted (not WorkerWake)"
    );
    assert!(
        !events.iter().any(|e| matches!(e, oj_core::Event::WorkerWake { .. })),
        "restart should NOT emit WorkerWake"
    );
}

#[test]
fn resolve_effective_project_path_uses_known_path_when_namespace_differs() {
    // Create two projects with different namespaces
    let project_a = tempdir().unwrap();
    let project_b = tempdir().unwrap();

    // Set up .oj directories for both
    std::fs::create_dir_all(project_a.path().join(".oj")).unwrap();
    std::fs::create_dir_all(project_b.path().join(".oj")).unwrap();

    // Configure project_b with a specific project "wok"
    std::fs::write(project_b.path().join(".oj/config.toml"), "[project]\nname = \"wok\"\n")
        .unwrap();

    // Set up state with a known worker from project_b project
    let mut initial_state = MaterializedState::default();
    initial_state.workers.insert(
        "wok/merge".to_string(),
        crate::storage::WorkerRecord {
            name: "merge".to_string(),
            project_path: project_b.path().to_path_buf(),
            runbook_hash: "hash".to_string(),
            status: "running".to_string(),
            active: vec![],
            queue: "merges".to_string(),
            concurrency: 1,
            owners: HashMap::new(),
            project: "wok".to_string(),
        },
    );
    let state = Arc::new(Mutex::new(initial_state));

    // When called with project_a's path but project "wok",
    // should resolve to project_b's path (the known root for "wok")
    let result = resolve_effective_project_path(project_a.path(), "wok", &state);

    assert_eq!(
        result,
        project_b.path().to_path_buf(),
        "should use known path for project 'wok', not the provided project_a path"
    );
}

#[test]
fn resolve_effective_project_path_uses_provided_path_when_namespace_matches() {
    // Create a project
    let project = tempdir().unwrap();
    std::fs::create_dir_all(project.path().join(".oj")).unwrap();

    // Configure project with project "myproject"
    std::fs::write(project.path().join(".oj/config.toml"), "[project]\nname = \"myproject\"\n")
        .unwrap();

    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // When called with matching project, should use provided root
    let result = resolve_effective_project_path(project.path(), "myproject", &state);

    assert_eq!(
        result,
        project.path().to_path_buf(),
        "should use provided path when project matches"
    );
}

#[test]
fn start_uses_known_path_when_namespace_differs_from_project_path() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

    // Create two projects: project_a (wrong one) and project_b (correct one with "wok" project)
    let project_a = tempdir().unwrap();
    let project_b = tempdir().unwrap();

    // Set up both projects with .oj directories
    std::fs::create_dir_all(project_a.path().join(".oj/runbooks")).unwrap();
    std::fs::create_dir_all(project_b.path().join(".oj/runbooks")).unwrap();

    // Configure project_b with project "wok"
    std::fs::write(project_b.path().join(".oj/config.toml"), "[project]\nname = \"wok\"\n")
        .unwrap();

    // Create a worker "merge" in project_b
    std::fs::write(
        project_b.path().join(".oj/runbooks/test.hcl"),
        r#"
queue "merges" {
  type = "persisted"
  vars = ["merge"]
}

worker "merge" {
  source  = { queue = "merges" }
  run = { job = "handle-merge" }
}

job "handle-merge" {
  step "run" {
    run = "echo merge"
  }
}
"#,
    )
    .unwrap();

    // Set up state with known project root for "wok" project
    {
        let mut state = ctx.state.lock();
        state.workers.insert(
            "wok/other-worker".to_string(),
            crate::storage::WorkerRecord {
                name: "other-worker".to_string(),
                project_path: project_b.path().to_path_buf(),
                runbook_hash: "hash".to_string(),
                status: "stopped".to_string(),
                active: vec![],
                queue: "other".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
                project: "wok".to_string(),
            },
        );
    }

    // Start worker with project_a's path but project "wok"
    // This simulates `oj --project wok worker start merge` from a different directory
    let result = handle_worker_start(&ctx, project_a.path(), "wok", "merge", false).unwrap();

    // Should succeed by using project_b's root (the known root for "wok")
    assert!(
        matches!(result, Response::WorkerStarted { ref worker } if worker == "merge"),
        "expected WorkerStarted for 'merge', got {:?}",
        result
    );
}

#[test]
fn start_already_running_worker_emits_wake_instead_of_started() {
    let dir = tempdir().unwrap();
    let (ctx, wal) = super::super::test_ctx_with_wal(dir.path());

    // Create a project with a worker
    let project = tempdir().unwrap();
    let runbook_dir = project.path().join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(
        runbook_dir.join("test.hcl"),
        r#"
queue "bugs" {
  list = "echo '[]'"
  take = "echo taken"
}

worker "fixer" {
  source  = { queue = "bugs" }
  run = { job = "build" }
  concurrency = 3
}

job "build" {
  step "run" {
    run = "echo build"
  }
}
"#,
    )
    .unwrap();

    // Put a running worker in state
    {
        let mut state = ctx.state.lock();
        state.workers.insert(
            "fixer".to_string(),
            crate::storage::WorkerRecord {
                name: "fixer".to_string(),
                project_path: project.path().to_path_buf(),
                runbook_hash: "old-hash".to_string(),
                status: "running".to_string(),
                active: vec![],
                queue: "bugs".to_string(),
                concurrency: 3,
                owners: HashMap::new(),
                project: String::new(),
            },
        );
    }

    // Call start on the already-running worker
    let result = handle_worker_start(&ctx, project.path(), "", "fixer", false).unwrap();

    // Response should still be WorkerStarted (preserving CLI contract)
    assert!(
        matches!(result, Response::WorkerStarted { ref worker } if worker == "fixer"),
        "expected WorkerStarted response, got {:?}",
        result
    );

    // But the emitted event should be WorkerWake, not WorkerStarted
    let events = drain_events(&wal);
    assert!(
        events.iter().any(|e| matches!(e, oj_core::Event::WorkerWake { .. })),
        "should emit WorkerWake for already-running worker, got events: {:?}",
        events.iter().map(|e| e.name()).collect::<Vec<_>>()
    );
    assert!(
        !events.iter().any(|e| matches!(e, oj_core::Event::WorkerStarted { .. })),
        "should NOT emit WorkerStarted for already-running worker"
    );
}

#[test]
fn stop_all_stops_running_workers_in_namespace() {
    let dir = tempdir().unwrap();
    let (ctx, wal) = super::super::test_ctx_with_wal(dir.path());

    // Insert two running workers and one stopped worker in project "proj"
    {
        let mut state = ctx.state.lock();
        state.workers.insert(
            "proj/alpha".to_string(),
            crate::storage::WorkerRecord {
                name: "alpha".to_string(),
                project_path: PathBuf::from("/fake"),
                runbook_hash: "hash".to_string(),
                status: "running".to_string(),
                active: vec![],
                queue: "q1".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
                project: "proj".to_string(),
            },
        );
        state.workers.insert(
            "proj/beta".to_string(),
            crate::storage::WorkerRecord {
                name: "beta".to_string(),
                project_path: PathBuf::from("/fake"),
                runbook_hash: "hash".to_string(),
                status: "running".to_string(),
                active: vec![],
                queue: "q2".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
                project: "proj".to_string(),
            },
        );
        state.workers.insert(
            "proj/gamma".to_string(),
            crate::storage::WorkerRecord {
                name: "gamma".to_string(),
                project_path: PathBuf::from("/fake"),
                runbook_hash: "hash".to_string(),
                status: "stopped".to_string(),
                active: vec![],
                queue: "q3".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
                project: "proj".to_string(),
            },
        );
        // Worker in a different project — should not be stopped
        state.workers.insert(
            "other/delta".to_string(),
            crate::storage::WorkerRecord {
                name: "delta".to_string(),
                project_path: PathBuf::from("/other"),
                runbook_hash: "hash".to_string(),
                status: "running".to_string(),
                active: vec![],
                queue: "q4".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
                project: "other".to_string(),
            },
        );
    }

    let result = handle_worker_stop(&ctx, "", "proj", None, true).unwrap();

    match result {
        Response::WorkersStopped { mut stopped, skipped } => {
            stopped.sort();
            assert_eq!(stopped, vec!["alpha", "beta"]);
            assert!(skipped.is_empty(), "expected no skipped, got {:?}", skipped);
        }
        other => panic!("expected WorkersStopped, got {:?}", other),
    }

    // Verify WorkerStopped events were emitted
    let events = drain_events(&wal);
    let stop_events: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            oj_core::Event::WorkerStopped { worker, .. } => Some(worker.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(stop_events.len(), 2, "expected 2 stop events, got {:?}", stop_events);
}

#[test]
fn stop_all_with_no_running_workers_returns_empty() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

    let result = handle_worker_stop(&ctx, "", "proj", None, true).unwrap();

    match result {
        Response::WorkersStopped { stopped, skipped } => {
            assert!(stopped.is_empty());
            assert!(skipped.is_empty());
        }
        other => panic!("expected WorkersStopped, got {:?}", other),
    }
}
