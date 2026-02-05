// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

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

    // Attempt startup -- should fail with LockFailed
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
