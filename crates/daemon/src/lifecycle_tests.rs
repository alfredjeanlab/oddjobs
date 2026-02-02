// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

use crate::event_bus::EventBus;
use oj_adapters::{ClaudeAgentAdapter, TmuxAdapter, TracedAgent, TracedSession};
use oj_core::{Event, PipelineConfig, PipelineId, StepStatus, SystemClock};
use oj_engine::{Runtime, RuntimeConfig, RuntimeDeps, Scheduler};
use oj_runbook::{PipelineDef, RunDirective, Runbook, StepDef};
use oj_storage::{MaterializedState, Wal};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;
use tokio::sync::mpsc;

/// Build a minimal runbook with a single-step pipeline.
fn test_runbook() -> Runbook {
    let mut pipelines = HashMap::new();
    pipelines.insert(
        "test".to_string(),
        PipelineDef {
            name: "test".to_string(),
            vars: vec![],
            defaults: HashMap::new(),
            locals: HashMap::new(),
            cwd: None,
            workspace: None,
            on_done: None,
            on_fail: None,
            notify: Default::default(),
            steps: vec![StepDef {
                name: "only-step".to_string(),
                run: RunDirective::Shell("echo done".to_string()),
                on_done: None,
                on_fail: None,
            }],
        },
    );
    Runbook {
        commands: HashMap::new(),
        pipelines,
        agents: HashMap::new(),
        queues: HashMap::new(),
        workers: HashMap::new(),
    }
}

/// Hash a runbook the same way the runtime does.
fn runbook_hash(runbook: &Runbook) -> String {
    let json = serde_json::to_value(runbook).unwrap();
    let canonical = serde_json::to_string(&json).unwrap();
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{:x}", digest)
}

/// Set up a DaemonState with a pipeline ready for step completion.
///
/// Returns the state and a WAL path for verification.
async fn setup_daemon_with_pipeline() -> (DaemonState, PathBuf) {
    let dir = tempdir().unwrap();
    let dir_path = dir.keep();

    let wal_path = dir_path.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, _event_reader) = EventBus::new(wal);

    // Build runbook and hash
    let runbook = test_runbook();
    let hash = runbook_hash(&runbook);
    let runbook_json = serde_json::to_value(&runbook).unwrap();

    // Pre-populate state with pipeline + stored runbook
    let mut state = MaterializedState::default();
    let config = PipelineConfig {
        id: "pipe-1".to_string(),
        name: "test-pipeline".to_string(),
        kind: "test".to_string(),
        vars: HashMap::new(),
        runbook_hash: hash.clone(),
        cwd: dir_path.clone(),
        initial_step: "only-step".to_string(),
        namespace: String::new(),
    };
    let pipeline = oj_core::Pipeline::new(config, &SystemClock);
    state.pipelines.insert("pipe-1".to_string(), pipeline);
    state.apply_event(&Event::RunbookLoaded {
        hash,
        version: 1,
        runbook: runbook_json,
    });

    // Mark pipeline step as running (as it would be during normal execution)
    state.pipelines.get_mut("pipe-1").unwrap().step_status = StepStatus::Running;

    let state = Arc::new(Mutex::new(state));
    let scheduler = Arc::new(Mutex::new(Scheduler::new()));

    // Create real adapters (won't be called for ShellExited → completion path)
    let session_adapter = TracedSession::new(TmuxAdapter::new());
    let agent_adapter = TracedAgent::new(ClaudeAgentAdapter::new(session_adapter.clone()));

    let (internal_tx, _internal_rx) = mpsc::channel::<Event>(100);
    let runtime = Arc::new(Runtime::new(
        RuntimeDeps {
            sessions: session_adapter,
            agents: agent_adapter,
            state: Arc::clone(&state),
        },
        SystemClock,
        RuntimeConfig {
            workspaces_root: dir_path.clone(),
            log_dir: dir_path.join("logs"),
        },
        internal_tx,
    ));

    let lock_path = dir_path.join("test.lock");
    let lock_file = std::fs::File::create(&lock_path).unwrap();

    let daemon = DaemonState {
        config: Config {
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
        scheduler,
        event_bus,
        start_time: std::time::Instant::now(),
    };

    (daemon, wal_path)
}

#[tokio::test]
async fn process_event_persists_result_events_to_wal() {
    let (mut daemon, wal_path) = setup_daemon_with_pipeline().await;

    // Send ShellExited which triggers advance_pipeline → completion
    // This produces PipelineAdvanced + StepUpdated result events
    daemon
        .process_event(Event::ShellExited {
            pipeline_id: PipelineId::new("pipe-1"),
            step: "only-step".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    // Flush the event bus to ensure events are written to disk
    daemon.event_bus.flush().unwrap();

    // Verify result events were persisted to WAL
    let wal = Wal::open(&wal_path, 0).unwrap();
    let entries = wal.entries_after(0).unwrap();

    // ShellExited → advance_pipeline (no next step) → step_transition "done" + completion
    // Expected result events: PipelineAdvanced("done"), StepUpdated(Completed)
    assert!(
        !entries.is_empty(),
        "result events should be persisted to WAL"
    );

    // Verify we have the expected event types
    let has_pipeline_updated = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::PipelineAdvanced { id, step } if id == "pipe-1" && step == "done"
        )
    });
    let has_step_completed = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::StepCompleted { pipeline_id, .. }
                if pipeline_id == "pipe-1"
        )
    });

    assert!(
        has_pipeline_updated,
        "PipelineAdvanced event should be in WAL"
    );
    assert!(has_step_completed, "StepCompleted event should be in WAL");
}

#[tokio::test]
async fn process_event_cancel_persists_to_wal() {
    let (mut daemon, wal_path) = setup_daemon_with_pipeline().await;

    // Cancel the pipeline via a typed event
    daemon
        .process_event(Event::PipelineCancel {
            id: PipelineId::new("pipe-1"),
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

    let has_pipeline_cancelled = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::PipelineAdvanced { id, step } if id == "pipe-1" && step == "cancelled"
        )
    });
    let has_step_failed = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::StepFailed { pipeline_id, .. }
                if pipeline_id == "pipe-1"
        )
    });

    assert!(
        has_pipeline_cancelled,
        "PipelineAdvanced(cancelled) should be in WAL"
    );
    assert!(has_step_failed, "StepFailed event should be in WAL");
}

#[tokio::test]
async fn cancelled_pipeline_survives_restart_as_terminal() {
    let (mut daemon, wal_path) = setup_daemon_with_pipeline().await;

    // Cancel the pipeline
    daemon
        .process_event(Event::PipelineCancel {
            id: PipelineId::new("pipe-1"),
        })
        .await
        .unwrap();

    daemon.event_bus.flush().unwrap();

    // Simulate daemon restart: build fresh state from WAL replay
    // In a real restart, the pipeline would come from a snapshot.
    // Here we recreate it manually to simulate the snapshot baseline.
    let mut recovered_state = MaterializedState::default();
    recovered_state.apply_event(&Event::PipelineCreated {
        id: PipelineId::new("pipe-1"),
        kind: "test".to_string(),
        name: "test-pipeline".to_string(),
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/tmp/test"),
        vars: HashMap::new(),
        initial_step: "only-step".to_string(),
        namespace: String::new(),
        created_at_epoch_ms: 1_000_000,
    });

    // Replay WAL events (as the daemon does on startup)
    let wal = Wal::open(&wal_path, 0).unwrap();
    let entries = wal.entries_after(0).unwrap();
    for entry in &entries {
        recovered_state.apply_event(&entry.event);
    }

    // Pipeline should be terminal after replay
    let pipeline = recovered_state.pipelines.get("pipe-1").unwrap();
    assert!(
        pipeline.is_terminal(),
        "cancelled pipeline should be terminal after WAL replay"
    );
    assert_eq!(pipeline.step, "cancelled");
    assert_eq!(pipeline.step_status, StepStatus::Failed);
}

#[tokio::test]
async fn process_event_materializes_state() {
    // Regression test: events from the WAL must be applied to MaterializedState
    // so that queries (e.g., ListWorkers) see them immediately.
    let (mut daemon, _wal_path) = setup_daemon_with_pipeline().await;

    // ShellExited should update pipeline step_status in MaterializedState
    daemon
        .process_event(Event::ShellExited {
            pipeline_id: PipelineId::new("pipe-1"),
            step: "only-step".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let state = daemon.state.lock();
    let pipeline = state.pipelines.get("pipe-1").unwrap();
    // The pipeline should have been advanced to "done" and be terminal
    assert!(
        pipeline.is_terminal(),
        "pipeline should be terminal after ShellExited(0) is processed"
    );
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
fn reconcile_context_counts_non_terminal_pipelines() {
    // Verify ReconcileContext.pipeline_count matches non-terminal pipelines.
    // This ensures background reconciliation knows how many pipelines to process.
    let mut state = MaterializedState::default();

    // Add a running pipeline (non-terminal)
    let mut running = oj_core::Pipeline::new(
        PipelineConfig {
            id: "pipe-running".to_string(),
            name: "test".to_string(),
            kind: "test".to_string(),
            vars: HashMap::new(),
            runbook_hash: "hash".to_string(),
            cwd: PathBuf::from("/tmp"),
            initial_step: "step".to_string(),
            namespace: String::new(),
        },
        &SystemClock,
    );
    running.step_status = StepStatus::Running;
    state.pipelines.insert("pipe-running".to_string(), running);

    // Add a completed pipeline (terminal)
    let mut done = oj_core::Pipeline::new(
        PipelineConfig {
            id: "pipe-done".to_string(),
            name: "test".to_string(),
            kind: "test".to_string(),
            vars: HashMap::new(),
            runbook_hash: "hash".to_string(),
            cwd: PathBuf::from("/tmp"),
            initial_step: "done".to_string(),
            namespace: String::new(),
        },
        &SystemClock,
    );
    done.step_status = StepStatus::Completed;
    state.pipelines.insert("pipe-done".to_string(), done);

    // Add a failed pipeline (terminal)
    let mut failed = oj_core::Pipeline::new(
        PipelineConfig {
            id: "pipe-failed".to_string(),
            name: "test".to_string(),
            kind: "test".to_string(),
            vars: HashMap::new(),
            runbook_hash: "hash".to_string(),
            cwd: PathBuf::from("/tmp"),
            initial_step: "failed".to_string(),
            namespace: String::new(),
        },
        &SystemClock,
    );
    failed.step_status = StepStatus::Failed;
    state.pipelines.insert("pipe-failed".to_string(), failed);

    // Count non-terminal pipelines (same logic as startup_inner)
    let pipeline_count = state
        .pipelines
        .values()
        .filter(|p| !p.is_terminal())
        .count();

    // Only the running pipeline is non-terminal
    assert_eq!(
        pipeline_count, 1,
        "only running pipeline should be counted as non-terminal"
    );
}

/// Helper to create a Config pointing at a temp directory.
fn test_config(dir: &Path) -> Config {
    Config {
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
