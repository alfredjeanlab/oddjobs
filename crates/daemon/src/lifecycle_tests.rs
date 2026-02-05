// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

use crate::event_bus::{EventBus, EventReader};
use oj_adapters::{
    ClaudeAgentAdapter, DesktopNotifyAdapter, TmuxAdapter, TracedAgent, TracedSession,
};
use oj_core::{
    AgentRun, AgentRunId, AgentRunStatus, Event, Job, JobConfig, JobId, StepOutcome, StepRecord,
    StepStatus, SystemClock,
};
use oj_engine::{Runtime, RuntimeConfig, RuntimeDeps};
use oj_runbook::{JobDef, RunDirective, Runbook, StepDef};
use oj_storage::{load_snapshot, MaterializedState, Wal, WorkerRecord};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;
use tokio::sync::mpsc;

/// Build a minimal runbook with a single-step job.
fn test_runbook() -> Runbook {
    let mut jobs = HashMap::new();
    jobs.insert(
        "test".to_string(),
        JobDef {
            kind: "test".to_string(),
            name: None,
            vars: vec![],
            defaults: HashMap::new(),
            locals: HashMap::new(),
            cwd: None,
            workspace: None,
            on_done: None,
            on_fail: None,
            on_cancel: None,
            notify: Default::default(),
            steps: vec![StepDef {
                name: "only-step".to_string(),
                run: RunDirective::Shell("echo done".to_string()),
                on_done: None,
                on_fail: None,
                on_cancel: None,
            }],
        },
    );
    Runbook {
        commands: HashMap::new(),
        jobs,
        agents: HashMap::new(),
        queues: HashMap::new(),
        workers: HashMap::new(),
        crons: HashMap::new(),
    }
}

/// Hash a runbook the same way the runtime does.
fn runbook_hash(runbook: &Runbook) -> String {
    let json = serde_json::to_value(runbook).unwrap();
    let canonical = serde_json::to_string(&json).unwrap();
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{:x}", digest)
}

/// Set up a DaemonState with a job ready for step completion.
///
/// Returns the state and a WAL path for verification.
async fn setup_daemon_with_job() -> (DaemonState, PathBuf) {
    let (daemon, _, wal_path) = setup_daemon_with_job_and_reader().await;
    (daemon, wal_path)
}

/// Like `setup_daemon_with_job` but also returns the EventReader
/// so callers can simulate the main loop (mark_processed, etc.).
async fn setup_daemon_with_job_and_reader() -> (DaemonState, EventReader, PathBuf) {
    let dir = tempdir().unwrap();
    let dir_path = dir.keep();

    let wal_path = dir_path.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, event_reader) = EventBus::new(wal);

    // Build runbook and hash
    let runbook = test_runbook();
    let hash = runbook_hash(&runbook);
    let runbook_json = serde_json::to_value(&runbook).unwrap();

    // Pre-populate state with job + stored runbook
    let mut state = MaterializedState::default();
    let config = JobConfig {
        id: "pipe-1".to_string(),
        name: "test-job".to_string(),
        kind: "test".to_string(),
        vars: HashMap::new(),
        runbook_hash: hash.clone(),
        cwd: dir_path.clone(),
        initial_step: "only-step".to_string(),
        namespace: String::new(),
        cron_name: None,
    };
    let job = oj_core::Job::new(config, &SystemClock);
    state.jobs.insert("pipe-1".to_string(), job);
    state.apply_event(&Event::RunbookLoaded {
        hash,
        version: 1,
        runbook: runbook_json,
    });

    // Mark job step as running (as it would be during normal execution)
    state.jobs.get_mut("pipe-1").unwrap().step_status = StepStatus::Running;

    let state = Arc::new(Mutex::new(state));

    // Create real adapters (won't be called for ShellExited → completion path)
    let session_adapter = TracedSession::new(TmuxAdapter::new());
    let agent_adapter = TracedAgent::new(ClaudeAgentAdapter::new(session_adapter.clone()));

    let (internal_tx, _internal_rx) = mpsc::channel::<Event>(100);
    let runtime = Arc::new(Runtime::new(
        RuntimeDeps {
            sessions: session_adapter,
            agents: agent_adapter,
            notifier: DesktopNotifyAdapter::new(),
            state: Arc::clone(&state),
        },
        SystemClock,
        RuntimeConfig {
            state_dir: dir_path.clone(),
            log_dir: dir_path.join("logs"),
        },
        internal_tx,
    ));

    let lock_path = dir_path.join("test.lock");
    let lock_file = std::fs::File::create(&lock_path).unwrap();

    let daemon = DaemonState {
        config: Config {
            state_dir: dir_path.clone(),
            socket_path: dir_path.join("test.sock"),
            lock_path,
            version_path: dir_path.join("test.version"),
            log_path: dir_path.join("test.log"),
            wal_path: wal_path.clone(),
            snapshot_path: dir_path.join("test.snapshot"),
            workspaces_path: dir_path.join("workspaces"),
            logs_path: dir_path.join("logs"),
        },
        lock_file,
        state,
        runtime,
        event_bus,
        start_time: std::time::Instant::now(),
        orphans: Arc::new(Mutex::new(Vec::new())),
    };

    (daemon, event_reader, wal_path)
}

#[tokio::test]
async fn process_event_persists_result_events_to_wal() {
    let (mut daemon, wal_path) = setup_daemon_with_job().await;

    // Send ShellExited which triggers advance_job → completion
    // This produces JobAdvanced + StepUpdated result events
    daemon
        .process_event(Event::ShellExited {
            job_id: JobId::new("pipe-1"),
            step: "only-step".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Flush the event bus to ensure events are written to disk
    daemon.event_bus.flush().unwrap();

    // Verify result events were persisted to WAL
    let wal = Wal::open(&wal_path, 0).unwrap();
    let entries = wal.entries_after(0).unwrap();

    // ShellExited → advance_job (no next step) → step_transition "done" + completion
    // Expected result events: JobAdvanced("done"), StepUpdated(Completed)
    assert!(
        !entries.is_empty(),
        "result events should be persisted to WAL"
    );

    // Verify we have the expected event types
    let has_job_updated = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::JobAdvanced { id, step } if id == "pipe-1" && step == "done"
        )
    });
    let has_step_completed = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::StepCompleted { job_id, .. }
                if job_id == "pipe-1"
        )
    });

    assert!(has_job_updated, "JobAdvanced event should be in WAL");
    assert!(has_step_completed, "StepCompleted event should be in WAL");
}

#[tokio::test]
async fn process_event_cancel_persists_to_wal() {
    let (mut daemon, wal_path) = setup_daemon_with_job().await;

    // Cancel the job via a typed event
    daemon
        .process_event(Event::JobCancel {
            id: JobId::new("pipe-1"),
        })
        .await
        .unwrap();

    // Flush the event bus to ensure events are written to disk
    daemon.event_bus.flush().unwrap();

    // Verify cancel events were persisted to WAL
    let wal = Wal::open(&wal_path, 0).unwrap();
    let entries = wal.entries_after(0).unwrap();

    assert!(
        !entries.is_empty(),
        "cancel events should be persisted to WAL"
    );

    let has_job_cancelled = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::JobAdvanced { id, step } if id == "pipe-1" && step == "cancelled"
        )
    });
    let has_step_failed = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::StepFailed { job_id, .. }
                if job_id == "pipe-1"
        )
    });

    assert!(has_job_cancelled, "JobAdvanced(cancelled) should be in WAL");
    assert!(has_step_failed, "StepFailed event should be in WAL");
}

#[tokio::test]
async fn cancelled_job_survives_restart_as_terminal() {
    let (mut daemon, wal_path) = setup_daemon_with_job().await;

    // Cancel the job
    daemon
        .process_event(Event::JobCancel {
            id: JobId::new("pipe-1"),
        })
        .await
        .unwrap();

    daemon.event_bus.flush().unwrap();

    // Simulate daemon restart: build fresh state from WAL replay
    // In a real restart, the job would come from a snapshot.
    // Here we recreate it manually to simulate the snapshot baseline.
    let mut recovered_state = MaterializedState::default();
    recovered_state.apply_event(&Event::JobCreated {
        id: JobId::new("pipe-1"),
        kind: "test".to_string(),
        name: "test-job".to_string(),
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/tmp/test"),
        vars: HashMap::new(),
        initial_step: "only-step".to_string(),
        namespace: String::new(),
        created_at_epoch_ms: 1_000_000,
        cron_name: None,
    });

    // Replay WAL events (as the daemon does on startup)
    let wal = Wal::open(&wal_path, 0).unwrap();
    let entries = wal.entries_after(0).unwrap();
    for entry in &entries {
        recovered_state.apply_event(&entry.event);
    }

    // Job should be terminal after replay
    let job = recovered_state.jobs.get("pipe-1").unwrap();
    assert!(
        job.is_terminal(),
        "cancelled job should be terminal after WAL replay"
    );
    assert_eq!(job.step, "cancelled");
    assert_eq!(job.step_status, StepStatus::Failed);
}

#[tokio::test]
async fn process_event_materializes_state() {
    // Regression test: events from the WAL must be applied to MaterializedState
    // so that queries (e.g., ListWorkers) see them immediately.
    let (mut daemon, _wal_path) = setup_daemon_with_job().await;

    // ShellExited should update job step_status in MaterializedState
    daemon
        .process_event(Event::ShellExited {
            job_id: JobId::new("pipe-1"),
            step: "only-step".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let state = daemon.state.lock();
    let job = state.jobs.get("pipe-1").unwrap();
    // The job should have been advanced to "done" and be terminal
    assert!(
        job.is_terminal(),
        "job should be terminal after ShellExited(0) is processed"
    );
}

#[tokio::test]
async fn result_events_delivered_once_through_engine_loop() {
    // Regression test for duplicate job creation (oj-3faca023).
    //
    // process_event must NOT re-process result events locally. Result events
    // are persisted to the WAL and processed by the engine loop on the next
    // iteration. Previously, process_event had a pending_events loop that
    // both persisted AND locally re-processed result events, causing handlers
    // to fire twice — e.g., WorkerPollComplete dispatching the same queue
    // item into two jobs.
    //
    // This test simulates the engine loop: process an event, read result
    // events from the WAL, process each, and verify the total event count
    // matches expectations (no duplicates from local re-processing).
    let (mut daemon, mut event_reader, _wal_path) = setup_daemon_with_job_and_reader().await;

    // Process ShellExited — produces StepCompleted + JobAdvanced result events
    daemon
        .process_event(Event::ShellExited {
            job_id: JobId::new("pipe-1"),
            step: "only-step".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    daemon.event_bus.flush().unwrap();

    // Simulate engine loop: read result events from WAL and process them
    let mut total_wal_events = 0usize;
    loop {
        let entry =
            match tokio::time::timeout(std::time::Duration::from_millis(50), event_reader.recv())
                .await
            {
                Ok(Ok(Some(entry))) => entry,
                _ => break,
            };
        event_reader.mark_processed(entry.seq);
        total_wal_events += 1;

        // Process the result event (as the engine loop would)
        daemon.process_event(entry.event).await.unwrap();
        daemon.event_bus.flush().unwrap();
    }

    // Read any secondary events produced by processing result events
    let mut secondary_events = 0usize;
    loop {
        match tokio::time::timeout(std::time::Duration::from_millis(50), event_reader.recv()).await
        {
            Ok(Ok(Some(entry))) => {
                event_reader.mark_processed(entry.seq);
                secondary_events += 1;
            }
            _ => break,
        }
    }

    // ShellExited → advance_job produces:
    //   1. StepCompleted (current step done)
    //   2. JobAdvanced("done") (from completion_effects)
    //   3. StepCompleted (from completion_effects)
    assert_eq!(
        total_wal_events, 3,
        "ShellExited should produce exactly 3 result events in WAL"
    );

    // JobAdvanced("done") handler returns empty (no worker tracking this
    // job), and StepCompleted has no handler. So no secondary events.
    assert_eq!(
        secondary_events, 0,
        "result event handlers should produce no secondary events (no worker)"
    );

    // Job should be terminal
    let state = daemon.state.lock();
    let job = state.jobs.get("pipe-1").unwrap();
    assert!(job.is_terminal());
}

#[test]
fn parking_lot_mutex_reentrant_lock_is_detected() {
    // parking_lot::Mutex does not allow re-entrant locking from the same thread.
    // When a lock is already held, try_lock() returns None immediately instead of
    // deadlocking. This lets us detect re-entrant lock attempts in tests and
    // debug scenarios, unlike std::sync::Mutex which silently deadlocks.
    let mutex = Mutex::new(42);
    let _guard = mutex.lock();
    assert!(
        mutex.try_lock().is_none(),
        "re-entrant lock on parking_lot::Mutex must fail (not silently deadlock)"
    );
}

#[test]
fn reconcile_context_counts_non_terminal_jobs() {
    // Verify ReconcileContext.job_count matches non-terminal jobs.
    // This ensures background reconciliation knows how many jobs to process.
    let mut state = MaterializedState::default();

    // Add a running job (non-terminal)
    let mut running = oj_core::Job::new(
        JobConfig {
            id: "pipe-running".to_string(),
            name: "test".to_string(),
            kind: "test".to_string(),
            vars: HashMap::new(),
            runbook_hash: "hash".to_string(),
            cwd: PathBuf::from("/tmp"),
            initial_step: "step".to_string(),
            namespace: String::new(),
            cron_name: None,
        },
        &SystemClock,
    );
    running.step_status = StepStatus::Running;
    state.jobs.insert("pipe-running".to_string(), running);

    // Add a completed job (terminal)
    let mut done = oj_core::Job::new(
        JobConfig {
            id: "pipe-done".to_string(),
            name: "test".to_string(),
            kind: "test".to_string(),
            vars: HashMap::new(),
            runbook_hash: "hash".to_string(),
            cwd: PathBuf::from("/tmp"),
            initial_step: "done".to_string(),
            namespace: String::new(),
            cron_name: None,
        },
        &SystemClock,
    );
    done.step_status = StepStatus::Completed;
    state.jobs.insert("pipe-done".to_string(), done);

    // Add a failed job (terminal)
    let mut failed = oj_core::Job::new(
        JobConfig {
            id: "pipe-failed".to_string(),
            name: "test".to_string(),
            kind: "test".to_string(),
            vars: HashMap::new(),
            runbook_hash: "hash".to_string(),
            cwd: PathBuf::from("/tmp"),
            initial_step: "failed".to_string(),
            namespace: String::new(),
            cron_name: None,
        },
        &SystemClock,
    );
    failed.step_status = StepStatus::Failed;
    state.jobs.insert("pipe-failed".to_string(), failed);

    // Count non-terminal jobs (same logic as startup_inner)
    let job_count = state.jobs.values().filter(|p| !p.is_terminal()).count();

    // Only the running job is non-terminal
    assert_eq!(
        job_count, 1,
        "only running job should be counted as non-terminal"
    );
}

/// Helper to create a Config pointing at a temp directory.
fn test_config(dir: &Path) -> Config {
    Config {
        state_dir: dir.to_path_buf(),
        socket_path: dir.join("test.sock"),
        lock_path: dir.join("test.lock"),
        version_path: dir.join("test.version"),
        log_path: dir.join("test.log"),
        wal_path: dir.join("test.wal"),
        snapshot_path: dir.join("test.snapshot"),
        workspaces_path: dir.join("workspaces"),
        logs_path: dir.join("logs"),
    }
}

#[tokio::test]
async fn startup_lock_failed_does_not_remove_existing_files() {
    // Simulate a running daemon by holding the lock and creating its files.
    // A second startup attempt must fail without deleting anything.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();

    let config = test_config(&dir_path);
    std::fs::create_dir_all(config.socket_path.parent().unwrap()).unwrap();

    // Create the files a running daemon would have
    std::fs::write(&config.socket_path, b"").unwrap();
    std::fs::write(&config.version_path, b"0.1.0").unwrap();

    // Hold an exclusive lock (simulating the running daemon)
    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&config.lock_path)
        .unwrap();
    use fs2::FileExt;
    lock_file.lock_exclusive().unwrap();
    std::fs::write(&config.lock_path, b"12345").unwrap();

    // Attempt startup — should fail with LockFailed
    match startup(&config).await {
        Err(LifecycleError::LockFailed(_)) => {} // expected
        Err(e) => panic!("expected LockFailed, got: {e}"),
        Ok(_) => panic!("expected LockFailed, but startup succeeded"),
    }

    // All files must still exist
    assert!(
        config.socket_path.exists(),
        "socket file must not be deleted on LockFailed"
    );
    assert!(
        config.version_path.exists(),
        "version file must not be deleted on LockFailed"
    );
    assert!(
        config.lock_path.exists(),
        "lock file must not be deleted on LockFailed"
    );
}

#[test]
fn lock_file_not_truncated_before_lock_acquired() {
    // Verify that opening the lock file for locking does not truncate it.
    // A running daemon's PID must survive another process opening the file.
    let dir = tempdir().unwrap();
    let lock_path = dir.path().join("test.lock");

    // Simulate running daemon: write PID and hold exclusive lock
    let running_lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    use fs2::FileExt;
    running_lock.lock_exclusive().unwrap();
    use std::io::Write;
    let mut f = &running_lock;
    writeln!(f, "99999").unwrap();

    // Second process opens the file (same OpenOptions as startup_inner)
    let _second = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();

    // PID written by the "running daemon" must still be readable
    let content = std::fs::read_to_string(&lock_path).unwrap();
    assert_eq!(
        content.trim(),
        "99999",
        "lock file content must not be truncated by another open"
    );
}

#[test]
fn cleanup_on_failure_removes_created_files() {
    // When startup fails for a non-lock reason (e.g. bind failure),
    // cleanup_on_failure should remove the files we created.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let config = test_config(&dir_path);

    // Create files as if startup_inner created them before failing
    std::fs::write(&config.socket_path, b"").unwrap();
    std::fs::write(&config.version_path, b"0.1.0").unwrap();
    std::fs::write(&config.lock_path, b"12345").unwrap();

    cleanup_on_failure(&config);

    assert!(
        !config.socket_path.exists(),
        "socket should be cleaned up on non-lock failure"
    );
    assert!(
        !config.version_path.exists(),
        "version file should be cleaned up on non-lock failure"
    );
    assert!(
        !config.lock_path.exists(),
        "lock file should be cleaned up on non-lock failure"
    );
}

#[tokio::test]
async fn reconcile_state_resumes_running_workers() {
    // Workers with status "running" should be re-emitted as WorkerStarted
    // events during reconciliation so the runtime recreates their in-memory
    // state and triggers an initial queue poll.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();

    let session_adapter = TracedSession::new(TmuxAdapter::new());
    let agent_adapter = TracedAgent::new(ClaudeAgentAdapter::new(session_adapter.clone()));
    let (internal_tx, _internal_rx) = mpsc::channel::<Event>(100);

    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let runtime = Arc::new(Runtime::new(
        RuntimeDeps {
            sessions: session_adapter.clone(),
            agents: agent_adapter,
            notifier: DesktopNotifyAdapter::new(),
            state: Arc::clone(&state),
        },
        SystemClock,
        RuntimeConfig {
            state_dir: dir_path.clone(),
            log_dir: dir_path.join("logs"),
        },
        internal_tx,
    ));

    // Build state with a running worker and a stopped worker
    let mut test_state = MaterializedState::default();
    test_state.workers.insert(
        "myns/running-worker".to_string(),
        WorkerRecord {
            name: "running-worker".to_string(),
            namespace: "myns".to_string(),
            project_root: dir_path.clone(),
            runbook_hash: "abc123".to_string(),
            status: "running".to_string(),
            active_job_ids: vec![],
            queue_name: "tasks".to_string(),
            concurrency: 2,
        },
    );
    test_state.workers.insert(
        "myns/stopped-worker".to_string(),
        WorkerRecord {
            name: "stopped-worker".to_string(),
            namespace: "myns".to_string(),
            project_root: dir_path.clone(),
            runbook_hash: "def456".to_string(),
            status: "stopped".to_string(),
            active_job_ids: vec![],
            queue_name: "other".to_string(),
            concurrency: 1,
        },
    );

    let (event_tx, mut event_rx) = mpsc::channel::<Event>(100);

    reconcile_state(&runtime, &test_state, &session_adapter, &event_tx).await;

    // Collect all emitted events
    drop(event_tx); // Close sender so recv() terminates
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Should have emitted exactly one WorkerStarted for the running worker
    let worker_started_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::WorkerStarted { .. }))
        .collect();
    assert_eq!(
        worker_started_events.len(),
        1,
        "should emit WorkerStarted for the one running worker, got: {:?}",
        worker_started_events
    );

    // Verify the event has the right fields
    match &worker_started_events[0] {
        Event::WorkerStarted {
            worker_name,
            project_root,
            runbook_hash,
            queue_name,
            concurrency,
            namespace,
        } => {
            assert_eq!(worker_name, "running-worker");
            assert_eq!(*project_root, dir_path);
            assert_eq!(runbook_hash, "abc123");
            assert_eq!(queue_name, "tasks");
            assert_eq!(*concurrency, 2);
            assert_eq!(namespace, "myns");
        }
        _ => unreachable!(),
    }
}

#[test]
fn reconcile_context_counts_running_workers() {
    let mut state = MaterializedState::default();

    state.workers.insert(
        "ns/w1".to_string(),
        WorkerRecord {
            name: "w1".to_string(),
            namespace: "ns".to_string(),
            project_root: PathBuf::from("/tmp"),
            runbook_hash: "hash".to_string(),
            status: "running".to_string(),
            active_job_ids: vec![],
            queue_name: "q".to_string(),
            concurrency: 1,
        },
    );
    state.workers.insert(
        "ns/w2".to_string(),
        WorkerRecord {
            name: "w2".to_string(),
            namespace: "ns".to_string(),
            project_root: PathBuf::from("/tmp"),
            runbook_hash: "hash".to_string(),
            status: "stopped".to_string(),
            active_job_ids: vec![],
            queue_name: "q".to_string(),
            concurrency: 1,
        },
    );

    // Same logic as startup_inner
    let worker_count = state
        .workers
        .values()
        .filter(|w| w.status == "running")
        .count();

    assert_eq!(worker_count, 1, "only running workers should be counted");
}

#[tokio::test]
async fn shutdown_saves_final_snapshot() {
    let (mut daemon, mut event_reader, _wal_path) = setup_daemon_with_job_and_reader().await;
    let snapshot_path = daemon.config.snapshot_path.clone();

    // Process an event so the WAL has entries
    daemon
        .process_event(Event::ShellExited {
            job_id: JobId::new("pipe-1"),
            step: "only-step".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Simulate the main loop: read events from WAL and mark processed
    daemon.event_bus.flush().unwrap();
    loop {
        match tokio::time::timeout(std::time::Duration::from_millis(50), event_reader.recv()).await
        {
            Ok(Ok(Some(entry))) => event_reader.mark_processed(entry.seq),
            _ => break,
        }
    }

    // No snapshot should exist yet
    assert!(
        !snapshot_path.exists(),
        "snapshot should not exist before shutdown"
    );

    // Shutdown should save a final snapshot
    daemon.shutdown().unwrap();

    assert!(
        snapshot_path.exists(),
        "shutdown must save a final snapshot"
    );

    // Verify the snapshot contains the correct state
    let snapshot = load_snapshot(&snapshot_path).unwrap().unwrap();
    assert!(
        snapshot.seq > 0,
        "snapshot seq should be non-zero after processing events"
    );
    let job = snapshot.state.jobs.get("pipe-1").unwrap();
    assert!(
        job.is_terminal(),
        "snapshot should contain the terminal job state"
    );
}

/// Helper to create a runtime for reconciliation tests.
fn setup_reconcile_runtime(dir_path: &Path) -> (Arc<DaemonRuntime>, TracedSession<TmuxAdapter>) {
    let session_adapter = TracedSession::new(TmuxAdapter::new());
    let agent_adapter = TracedAgent::new(ClaudeAgentAdapter::new(session_adapter.clone()));
    let (internal_tx, _internal_rx) = mpsc::channel::<Event>(100);

    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let runtime = Arc::new(Runtime::new(
        RuntimeDeps {
            sessions: session_adapter.clone(),
            agents: agent_adapter,
            notifier: DesktopNotifyAdapter::new(),
            state: Arc::clone(&state),
        },
        SystemClock,
        RuntimeConfig {
            state_dir: dir_path.to_path_buf(),
            log_dir: dir_path.join("logs"),
        },
        internal_tx,
    ));

    (runtime, session_adapter)
}

/// Helper to create a job with an agent_id in step_history and a session_id.
fn make_job_with_agent(id: &str, step: &str, agent_uuid: &str, session_id: &str) -> Job {
    Job {
        id: id.to_string(),
        name: "test-job".to_string(),
        kind: "test".to_string(),
        namespace: "proj".to_string(),
        step: step.to_string(),
        step_status: StepStatus::Running,
        step_started_at: std::time::Instant::now(),
        step_history: vec![StepRecord {
            name: step.to_string(),
            started_at_ms: 1000,
            finished_at_ms: None,
            outcome: StepOutcome::Running,
            agent_id: Some(agent_uuid.to_string()),
            agent_name: Some("test-agent".to_string()),
        }],
        vars: HashMap::new(),
        runbook_hash: "abc123".to_string(),
        cwd: PathBuf::from("/tmp/project"),
        workspace_id: None,
        workspace_path: None,
        session_id: Some(session_id.to_string()),
        created_at: std::time::Instant::now(),
        error: None,
        action_tracker: Default::default(),
        cancelling: false,
        total_retries: 0,
        step_visits: HashMap::new(),
        cron_name: None,
        idle_grace_log_size: None,
        last_nudge_at: None,
    }
}

#[tokio::test]
async fn reconcile_job_dead_session_uses_step_history_agent_id() {
    // When a job's tmux session is dead, reconciliation should emit
    // AgentGone with the agent_id from step_history (a UUID), not a
    // fabricated "{job_id}-{step}" string.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let (runtime, session_adapter) = setup_reconcile_runtime(&dir_path);

    let agent_uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let mut test_state = MaterializedState::default();
    test_state.jobs.insert(
        "pipe-1".to_string(),
        make_job_with_agent("pipe-1", "build", agent_uuid, "oj-nonexistent-session"),
    );

    let (event_tx, mut event_rx) = mpsc::channel::<Event>(100);
    reconcile_state(&runtime, &test_state, &session_adapter, &event_tx).await;

    drop(event_tx);
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Should emit AgentGone with the UUID from step_history
    let gone_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::AgentGone { .. }))
        .collect();
    assert_eq!(
        gone_events.len(),
        1,
        "should emit exactly one AgentGone event"
    );

    match &gone_events[0] {
        Event::AgentGone { agent_id, .. } => {
            assert_eq!(
                agent_id.as_str(),
                agent_uuid,
                "AgentGone must use UUID from step_history, not job_id-step"
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn reconcile_job_no_agent_id_in_step_history_skips() {
    // When a job has no agent_id in step_history (e.g., shell step
    // or crashed before agent was recorded), reconciliation should skip
    // it rather than emitting events with fabricated agent_ids.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let (runtime, session_adapter) = setup_reconcile_runtime(&dir_path);

    let mut job = make_job_with_agent("pipe-2", "work", "any", "oj-nonexistent");
    // Clear agent_id from step_history
    job.step_history[0].agent_id = None;

    let mut test_state = MaterializedState::default();
    test_state.jobs.insert("pipe-2".to_string(), job);

    let (event_tx, mut event_rx) = mpsc::channel::<Event>(100);
    reconcile_state(&runtime, &test_state, &session_adapter, &event_tx).await;

    drop(event_tx);
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Should not emit any agent events for this job
    let agent_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::AgentGone { .. } | Event::AgentExited { .. }))
        .collect();
    assert!(
        agent_events.is_empty(),
        "should not emit agent events when step_history has no agent_id, got: {:?}",
        agent_events
    );
}

#[tokio::test]
async fn reconcile_agent_run_dead_session_emits_gone_with_correct_id() {
    // When an agent run's tmux session is dead, reconciliation should
    // emit AgentGone with the agent_id from the agent_run record.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let (runtime, session_adapter) = setup_reconcile_runtime(&dir_path);

    let agent_uuid = "deadbeef-1234-5678-9abc-def012345678";
    let mut test_state = MaterializedState::default();
    test_state.agent_runs.insert(
        "ar-1".to_string(),
        AgentRun {
            id: "ar-1".to_string(),
            agent_name: "test-agent".to_string(),
            command_name: "do-work".to_string(),
            namespace: "proj".to_string(),
            cwd: dir_path.clone(),
            runbook_hash: "hash123".to_string(),
            status: AgentRunStatus::Running,
            agent_id: Some(agent_uuid.to_string()),
            session_id: Some("oj-nonexistent-ar-session".to_string()),
            error: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
            action_tracker: Default::default(),
            vars: HashMap::new(),
            idle_grace_log_size: None,
            last_nudge_at: None,
        },
    );

    let (event_tx, mut event_rx) = mpsc::channel::<Event>(100);
    reconcile_state(&runtime, &test_state, &session_adapter, &event_tx).await;

    drop(event_tx);
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Should emit AgentGone with the correct UUID
    let gone_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::AgentGone { .. }))
        .collect();
    assert_eq!(
        gone_events.len(),
        1,
        "should emit exactly one AgentGone event"
    );
    match &gone_events[0] {
        Event::AgentGone { agent_id, .. } => {
            assert_eq!(agent_id.as_str(), agent_uuid);
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn reconcile_agent_run_no_agent_id_marks_failed_directly() {
    // When an agent run has no agent_id (daemon crashed before
    // AgentRunStarted was persisted), reconciliation should directly
    // emit AgentRunStatusChanged(Failed) instead of trying to route
    // through AgentExited/AgentGone events that would be dropped.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let (runtime, session_adapter) = setup_reconcile_runtime(&dir_path);

    let mut test_state = MaterializedState::default();
    test_state.agent_runs.insert(
        "ar-2".to_string(),
        AgentRun {
            id: "ar-2".to_string(),
            agent_name: "test-agent".to_string(),
            command_name: "do-work".to_string(),
            namespace: "proj".to_string(),
            cwd: dir_path.clone(),
            runbook_hash: "hash123".to_string(),
            status: AgentRunStatus::Starting,
            agent_id: None, // No agent_id yet
            session_id: Some("oj-nonexistent-ar-session".to_string()),
            error: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
            action_tracker: Default::default(),
            vars: HashMap::new(),
            idle_grace_log_size: None,
            last_nudge_at: None,
        },
    );

    let (event_tx, mut event_rx) = mpsc::channel::<Event>(100);
    reconcile_state(&runtime, &test_state, &session_adapter, &event_tx).await;

    drop(event_tx);
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Should emit AgentRunStatusChanged(Failed) directly
    let status_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::AgentRunStatusChanged { .. }))
        .collect();
    assert_eq!(
        status_events.len(),
        1,
        "should emit exactly one AgentRunStatusChanged event"
    );
    match &status_events[0] {
        Event::AgentRunStatusChanged { id, status, reason } => {
            assert_eq!(id, &AgentRunId::new("ar-2"));
            assert_eq!(*status, AgentRunStatus::Failed);
            assert!(
                reason.as_ref().unwrap().contains("no agent_id"),
                "reason should mention missing agent_id, got: {:?}",
                reason
            );
        }
        _ => unreachable!(),
    }

    // Should NOT emit AgentGone or AgentExited
    let agent_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::AgentGone { .. } | Event::AgentExited { .. }))
        .collect();
    assert!(
        agent_events.is_empty(),
        "should not emit AgentGone/AgentExited when agent_id is None"
    );
}

#[tokio::test]
async fn reconcile_agent_run_no_session_id_marks_failed() {
    // When an agent run has no session_id, reconciliation should
    // directly emit AgentRunStatusChanged(Failed).
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let (runtime, session_adapter) = setup_reconcile_runtime(&dir_path);

    let mut test_state = MaterializedState::default();
    test_state.agent_runs.insert(
        "ar-3".to_string(),
        AgentRun {
            id: "ar-3".to_string(),
            agent_name: "test-agent".to_string(),
            command_name: "do-work".to_string(),
            namespace: "proj".to_string(),
            cwd: dir_path.clone(),
            runbook_hash: "hash123".to_string(),
            status: AgentRunStatus::Running,
            agent_id: Some("some-uuid".to_string()),
            session_id: None, // No session
            error: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
            action_tracker: Default::default(),
            vars: HashMap::new(),
            idle_grace_log_size: None,
            last_nudge_at: None,
        },
    );

    let (event_tx, mut event_rx) = mpsc::channel::<Event>(100);
    reconcile_state(&runtime, &test_state, &session_adapter, &event_tx).await;

    drop(event_tx);
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    let status_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::AgentRunStatusChanged { .. }))
        .collect();
    assert_eq!(status_events.len(), 1);
    match &status_events[0] {
        Event::AgentRunStatusChanged { id, status, reason } => {
            assert_eq!(id, &AgentRunId::new("ar-3"));
            assert_eq!(*status, AgentRunStatus::Failed);
            assert!(reason.as_ref().unwrap().contains("no session"));
        }
        _ => unreachable!(),
    }
}
