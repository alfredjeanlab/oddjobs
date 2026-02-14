// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn worker_started_with_queue_and_concurrency() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::WorkerStarted {
        worker: "fixer".to_string(),
        project_path: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        queue: "bugs".to_string(),
        concurrency: 3,
        project: String::new(),
    });
    let worker = &state.workers["fixer"];
    assert_eq!(worker.status, "running");
    assert_eq!(worker.queue, "bugs");
    assert_eq!(worker.concurrency, 3);
    assert!(worker.active.is_empty());
}

#[test]
fn worker_stopped_sets_status() {
    let mut state = MaterializedState::default();
    state.apply_event(&worker_start_event("fixer", ""));
    assert_eq!(state.workers["fixer"].status, "running");

    state
        .apply_event(&Event::WorkerStopped { worker: "fixer".to_string(), project: String::new() });
    assert_eq!(state.workers["fixer"].status, "stopped");
}

#[yare::parameterized(
    no_namespace   = { "", "fixer" },
    with_namespace = { "myproject", "myproject/fixer" },
)]
fn worker_started_preserves_active_job_ids(ns: &str, worker_key: &str) {
    let mut state = MaterializedState::default();

    state.apply_event(&worker_start_event("fixer", ns));
    state.apply_event(&Event::WorkerDispatched {
        worker: "fixer".to_string(),
        item_id: "item-1".to_string(),
        owner: JobId::from_string("job-1").into(),
        project: ns.to_string(),
    });
    state.apply_event(&Event::WorkerDispatched {
        worker: "fixer".to_string(),
        item_id: "item-2".to_string(),
        owner: JobId::from_string("job-2").into(),
        project: ns.to_string(),
    });

    assert_eq!(state.workers[worker_key].active.len(), 2);

    // Simulate daemon restart: WorkerStarted replayed from WAL
    state.apply_event(&worker_start_event("fixer", ns));

    let worker = &state.workers[worker_key];
    assert_eq!(worker.active.len(), 2);
    assert!(worker.active.contains(&"job-1".to_string()));
    assert!(worker.active.contains(&"job-2".to_string()));
}

#[test]
fn worker_deleted_lifecycle_and_ghost() {
    let mut state = MaterializedState::default();

    // Namespaced worker: start → stop → delete
    state.apply_event(&worker_start_event("fixer", "myproject"));
    assert_eq!(state.workers["myproject/fixer"].status, "running");
    state.apply_event(&Event::WorkerStopped {
        worker: "fixer".to_string(),
        project: "myproject".to_string(),
    });
    assert_eq!(state.workers["myproject/fixer"].status, "stopped");
    state.apply_event(&Event::WorkerDeleted {
        worker: "fixer".to_string(),
        project: "myproject".to_string(),
    });
    assert!(!state.workers.contains_key("myproject/fixer"));

    // Ghost worker (empty project): start → delete
    state.apply_event(&worker_start_event("ghost", ""));
    assert!(state.workers.contains_key("ghost"));
    state
        .apply_event(&Event::WorkerDeleted { worker: "ghost".to_string(), project: String::new() });
    assert!(!state.workers.contains_key("ghost"));

    // Delete nonexistent worker is a no-op
    state.apply_event(&Event::WorkerDeleted {
        worker: "nonexistent".to_string(),
        project: String::new(),
    });
    assert!(state.workers.is_empty());
}
