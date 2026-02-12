// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::test_helpers::*;

#[tokio::test]
async fn process_event_persists_result_events_to_wal() {
    let (mut daemon, wal_path) = setup_daemon_with_job().await;

    // Send ShellExited which triggers advance_job -> completion
    // This produces JobAdvanced + StepUpdated result events
    daemon
        .process_event(Event::ShellExited {
            job_id: JobId::new("job-1"),
            step: "only-step".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Flush the event bus to ensure events are written to disk
    daemon.event_bus.wal.lock().flush().unwrap();

    // Verify result events were persisted to WAL
    let wal = Wal::open(&wal_path, 0).unwrap();
    let entries = wal.entries_after(0).unwrap();

    // ShellExited -> advance_job (no next step) -> step_transition "done" + completion
    // Expected result events: JobAdvanced("done"), StepUpdated(Completed)
    assert!(!entries.is_empty(), "result events should be persisted to WAL");

    // Verify we have the expected event types
    let has_job_updated = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::JobAdvanced { id, step } if id == "job-1" && step == "done"
        )
    });
    let has_step_completed = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::StepCompleted { job_id, .. }
                if job_id == "job-1"
        )
    });

    assert!(has_job_updated, "JobAdvanced event should be in WAL");
    assert!(has_step_completed, "StepCompleted event should be in WAL");
}

#[tokio::test]
async fn process_event_cancel_persists_to_wal() {
    let (mut daemon, wal_path) = setup_daemon_with_job().await;

    // Cancel the job via a typed event
    daemon.process_event(Event::JobCancel { id: JobId::new("job-1") }).await.unwrap();

    // Flush the event bus to ensure events are written to disk
    daemon.event_bus.wal.lock().flush().unwrap();

    // Verify cancel events were persisted to WAL
    let wal = Wal::open(&wal_path, 0).unwrap();
    let entries = wal.entries_after(0).unwrap();

    assert!(!entries.is_empty(), "cancel events should be persisted to WAL");

    let has_job_cancelled = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::JobAdvanced { id, step } if id == "job-1" && step == "cancelled"
        )
    });
    let has_step_failed = entries.iter().any(|e| {
        matches!(
            &e.event,
            Event::StepFailed { job_id, .. }
                if job_id == "job-1"
        )
    });

    assert!(has_job_cancelled, "JobAdvanced(cancelled) should be in WAL");
    assert!(has_step_failed, "StepFailed event should be in WAL");
}

#[tokio::test]
async fn cancelled_job_survives_restart_as_terminal() {
    let (mut daemon, wal_path) = setup_daemon_with_job().await;

    // Cancel the job
    daemon.process_event(Event::JobCancel { id: JobId::new("job-1") }).await.unwrap();

    daemon.event_bus.wal.lock().flush().unwrap();

    // Simulate daemon restart: build fresh state from WAL replay
    // In a real restart, the job would come from a snapshot.
    // Here we recreate it manually to simulate the snapshot baseline.
    let mut recovered_state = MaterializedState::default();
    recovered_state.apply_event(&Event::JobCreated {
        id: JobId::new("job-1"),
        kind: "test".to_string(),
        name: "test-job".to_string(),
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/tmp/test"),
        vars: HashMap::new(),
        initial_step: "only-step".to_string(),
        project: String::new(),
        created_at_ms: 1_000_000,
        cron: None,
    });

    // Replay WAL events (as the daemon does on startup)
    let wal = Wal::open(&wal_path, 0).unwrap();
    let entries = wal.entries_after(0).unwrap();
    for entry in &entries {
        recovered_state.apply_event(&entry.event);
    }

    // Job should be terminal after replay
    let job = recovered_state.jobs.get("job-1").unwrap();
    assert!(job.is_terminal(), "cancelled job should be terminal after WAL replay");
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
            job_id: JobId::new("job-1"),
            step: "only-step".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let state = daemon.state.lock();
    let job = state.jobs.get("job-1").unwrap();
    // The job should have been advanced to "done" and be terminal
    assert!(job.is_terminal(), "job should be terminal after ShellExited(0) is processed");
}

#[tokio::test]
async fn result_events_delivered_once_through_engine_loop() {
    // Regression test for duplicate job creation (oj-3faca023).
    //
    // process_event must NOT re-process result events locally. Result events
    // are persisted to the WAL and processed by the engine loop on the next
    // iteration. Previously, process_event had a pending_events loop that
    // both persisted AND locally re-processed result events, causing handlers
    // to fire twice -- e.g., WorkerPolled dispatching the same queue
    // item into two jobs.
    //
    // This test simulates the engine loop: process an event, read result
    // events from the WAL, process each, and verify the total event count
    // matches expectations (no duplicates from local re-processing).
    let (mut daemon, mut event_reader, _wal_path) = setup_daemon_with_job_and_reader().await;

    // Process ShellExited -- produces StepCompleted + JobAdvanced result events
    daemon
        .process_event(Event::ShellExited {
            job_id: JobId::new("job-1"),
            step: "only-step".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    daemon.event_bus.wal.lock().flush().unwrap();

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
        event_reader.wal.lock().mark_processed(entry.seq);
        total_wal_events += 1;

        // Process the result event (as the engine loop would)
        daemon.process_event(entry.event).await.unwrap();
        daemon.event_bus.wal.lock().flush().unwrap();
    }

    // Read any secondary events produced by processing result events
    let mut secondary_events = 0usize;
    loop {
        match tokio::time::timeout(std::time::Duration::from_millis(50), event_reader.recv()).await
        {
            Ok(Ok(Some(entry))) => {
                event_reader.wal.lock().mark_processed(entry.seq);
                secondary_events += 1;
            }
            _ => break,
        }
    }

    // ShellExited -> advance_job produces:
    //   1. StepCompleted (current step done)
    //   2. JobAdvanced("done") (from completion_effects)
    //   3. StepCompleted (from completion_effects)
    assert_eq!(total_wal_events, 3, "ShellExited should produce exactly 3 result events in WAL");

    // JobAdvanced("done") handler returns empty (no worker tracking this
    // job), and StepCompleted has no handler. So no secondary events.
    assert_eq!(
        secondary_events, 0,
        "result event handlers should produce no secondary events (no worker)"
    );

    // Job should be terminal
    let state = daemon.state.lock();
    let job = state.jobs.get("job-1").unwrap();
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

#[tokio::test]
async fn shutdown_saves_final_snapshot() {
    let (mut daemon, mut event_reader, _wal_path) = setup_daemon_with_job_and_reader().await;
    let snapshot_path = daemon.config.snapshot_path.clone();

    // Process an event so the WAL has entries
    daemon
        .process_event(Event::ShellExited {
            job_id: JobId::new("job-1"),
            step: "only-step".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Simulate the main loop: read events from WAL and mark processed
    daemon.event_bus.wal.lock().flush().unwrap();
    loop {
        match tokio::time::timeout(std::time::Duration::from_millis(50), event_reader.recv()).await
        {
            Ok(Ok(Some(entry))) => event_reader.wal.lock().mark_processed(entry.seq),
            _ => break,
        }
    }

    // No snapshot should exist yet
    assert!(!snapshot_path.exists(), "snapshot should not exist before shutdown");

    // Shutdown should save a final snapshot
    daemon.shutdown().unwrap();

    assert!(snapshot_path.exists(), "shutdown must save a final snapshot");

    // Verify the snapshot contains the correct state
    let snapshot = load_snapshot(&snapshot_path).unwrap().unwrap();
    assert!(snapshot.seq > 0, "snapshot seq should be non-zero after processing events");
    let job = snapshot.state.jobs.get("job-1").unwrap();
    assert!(job.is_terminal(), "snapshot should contain the terminal job state");
}
