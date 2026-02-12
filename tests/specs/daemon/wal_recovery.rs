//! WAL recovery specs
//!
//! Verify that crash recovery with WAL replay and snapshots works correctly
//! under load and during snapshot creation.

use crate::prelude::*;

/// Runbook with a worker that processes many items to generate high event load.
/// Based on the working pattern from concurrency.rs tests.
///
/// Includes retry config so that orphaned items (jobs lost during crash) get
/// retried after daemon recovery instead of going straight to Dead status.
const HIGH_LOAD_RUNBOOK: &str = r#"
[queue.tasks]
type = "persisted"
vars = ["cmd"]
retry = { attempts = 3, cooldown = "0s" }

[worker.processor]
run = { job = "process" }
source = { queue = "tasks" }
concurrency = 4

[job.process]
vars = ["task"]

[[job.process.step]]
name = "work"
run = "${var.task.cmd}"
"#;

/// Tests recovery after daemon crash with many events in the WAL.
///
/// Scenario:
/// 1. Start daemon and push many queue items (generating many WAL events)
/// 2. Wait for all items to complete (ensures WAL has many durable events)
/// 3. Kill daemon with SIGKILL (crash simulation)
/// 4. Restart daemon (triggers snapshot + WAL replay recovery)
/// 5. Verify recovered state matches pre-crash state
///
/// Waiting for all items to complete before crashing avoids a race between
/// in-memory state updates and the async WAL flush task (~10ms interval).
/// If we crash mid-flight, items visible as "completed" in memory may not
/// yet be flushed to the WAL, leading to state loss on recovery.
#[test]
fn recovers_state_correctly_after_crash_with_many_events() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/load.toml", HIGH_LOAD_RUNBOOK);

    // Start daemon and worker
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "processor"]).passes();

    // Push 20 items to generate many events (each item creates multiple events:
    // QueueItemCreated, ItemDispatched, JobCreated, StepStarted, etc.)
    for i in 0..20 {
        temp.oj()
            .args(&["queue", "push", "tasks", &format!(r#"{{"cmd": "echo item-{}"}}"#, i)])
            .passes();
    }

    // Wait for ALL items to complete before crashing. This ensures the WAL
    // contains many durable events (the ~10ms flush interval will have run
    // many times during processing) without racing the flush task.
    let all_processed = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        out.matches("completed").count() >= 20
    });

    if !all_processed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        let items = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        eprintln!("=== QUEUE STATE ===\n{}\n=== END QUEUE ===", items);
    }
    assert!(all_processed, "all items should complete before crash");

    // Kill daemon with SIGKILL (simulates crash - no graceful shutdown)
    let killed = temp.daemon_kill();
    assert!(killed, "should be able to kill daemon");

    // Wait for daemon to actually die. Use .command() instead of .passes()
    // because after SIGKILL the stale socket may cause "Connection closed"
    // errors (exit code 1) which would panic inside the wait_for closure.
    let daemon_dead = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["daemon", "status"]).command().output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        !stdout.contains("Status: running")
    });
    assert!(daemon_dead, "daemon should be dead after kill");

    // Restart daemon - triggers recovery via snapshot + WAL replay
    temp.oj().args(&["daemon", "start"]).passes();

    // Verify all items are still completed after recovery.
    // No worker restart needed — all items were already done pre-crash.
    let recovered = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
    let completed = recovered.matches("completed").count();

    if completed < 20 {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        eprintln!("=== QUEUE STATE ===\n{}\n=== END QUEUE ===", recovered);
    }
    assert!(completed >= 20, "all 20 items should be recovered as completed, got {}", completed,);
}

/// Tests that daemon recovers correctly when a .tmp snapshot file exists
/// (simulating a crash during snapshot creation).
///
/// The checkpoint save is atomic: write to .tmp, fsync, rename.
/// A crash during write leaves a .tmp file. On restart, the daemon should:
/// - Ignore the incomplete .tmp file
/// - Use the previous valid snapshot (if any) + WAL replay
#[test]
fn recovers_when_tmp_snapshot_exists_from_interrupted_save() {
    use std::io::Write;

    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/load.toml", HIGH_LOAD_RUNBOOK);

    // Start daemon and create some state
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "processor"]).passes();

    // Push a few items and wait for completion
    for i in 0..5 {
        temp.oj()
            .args(&["queue", "push", "tasks", &format!(r#"{{"cmd": "echo pre-crash-{}"}}"#, i)])
            .passes();
    }

    let items_done = wait_for(SPEC_WAIT_MAX_MS * 2, || {
        let out = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        out.matches("completed").count() >= 5
    });
    assert!(items_done, "items should complete before crash test");

    // Gracefully stop daemon (this saves a valid snapshot)
    temp.oj().args(&["daemon", "stop"]).passes();

    // Wait for daemon to fully stop
    let stopped = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["daemon", "status"]).passes().stdout().contains("not running")
    });
    assert!(stopped, "daemon should stop");

    // Simulate interrupted snapshot save by creating a .tmp file
    // This mimics a crash during the "write to .tmp" phase
    let tmp_path = temp.state_path().join("snapshot.tmp");
    {
        let mut file = std::fs::File::create(&tmp_path).unwrap();
        // Write partial/invalid content (simulating incomplete write)
        file.write_all(b"INCOMPLETE_SNAPSHOT_DATA").unwrap();
        file.sync_all().unwrap();
    }

    // Restart daemon - should recover despite the .tmp file
    temp.oj().args(&["daemon", "start"]).passes();

    // Verify the daemon started successfully and state is preserved
    temp.oj().args(&["daemon", "status"]).passes().stdout_has("Status: running");

    // Verify previous state is preserved (the 5 completed items)
    let recovered_items = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
    let completed_count = recovered_items.matches("completed").count();
    assert!(
        completed_count >= 5,
        "should have recovered at least 5 completed items, got {}\n{}",
        completed_count,
        recovered_items
    );
}

/// Tests that daemon fails with a clear error when the snapshot file is corrupt.
///
/// When the snapshot fails to decompress (invalid zstd), the daemon should fail
/// with a clear error message so users know to delete or move the snapshot.
#[test]
fn corrupt_snapshot_produces_clear_error() {
    use std::io::Write;

    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/load.toml", HIGH_LOAD_RUNBOOK);

    // Create a corrupt snapshot file before starting daemon
    let snapshot_path = temp.state_path().join("snapshot.json");
    std::fs::create_dir_all(temp.state_path()).unwrap();
    {
        let mut file = std::fs::File::create(&snapshot_path).unwrap();
        // Write content that looks like zstd but is invalid
        // (real zstd magic number but garbage payload)
        file.write_all(b"\x28\xb5\x2f\xfd\x00\x00CORRUPT_DATA_HERE").unwrap();
        file.sync_all().unwrap();
    }

    // Daemon should fail with a clear error
    temp.oj()
        .args(&["daemon", "start"])
        .fails()
        .stderr_has("Snapshot error")
        .stderr_lacks("Connection timeout");
}

/// Tests that multiple crash-recovery cycles don't corrupt state.
///
/// This verifies that the WAL and snapshot system handles repeated
/// crashes gracefully without accumulating corruption. Each cycle
/// waits for all items to complete before crashing to avoid racing
/// the async WAL flush task.
#[test]
fn multiple_crash_recovery_cycles_preserve_state() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/load.toml", HIGH_LOAD_RUNBOOK);

    // === Cycle 1: Push items, process all, crash ===
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "processor"]).passes();

    for i in 0..5 {
        temp.oj()
            .args(&["queue", "push", "tasks", &format!(r#"{{"cmd": "echo cycle1-{}"}}"#, i)])
            .passes();
    }

    let cycle1_done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        out.matches("completed").count() >= 5
    });
    assert!(cycle1_done, "cycle 1: all 5 items should complete");

    // Crash #1
    let killed1 = temp.daemon_kill();
    assert!(killed1, "should kill daemon #1");

    let dead1 = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["daemon", "status"]).command().output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        !stdout.contains("Status: running")
    });
    assert!(dead1, "daemon #1 should be dead");

    // === Cycle 2: Recover, add more work, process all, crash ===
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "processor"]).passes();

    for i in 0..5 {
        temp.oj()
            .args(&["queue", "push", "tasks", &format!(r#"{{"cmd": "echo cycle2-{}"}}"#, i)])
            .passes();
    }

    // All 10 items (5 from cycle 1 recovered + 5 new) should complete
    let cycle2_done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        out.matches("completed").count() >= 10
    });
    assert!(cycle2_done, "cycle 2: all 10 items should complete");

    // Crash #2
    let killed2 = temp.daemon_kill();
    assert!(killed2, "should kill daemon #2");

    let dead2 = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["daemon", "status"]).command().output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        !stdout.contains("Status: running")
    });
    assert!(dead2, "daemon #2 should be dead");

    // === Cycle 3: Final recovery, add more work, verify everything completes ===
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "processor"]).passes();

    for i in 0..5 {
        temp.oj()
            .args(&["queue", "push", "tasks", &format!(r#"{{"cmd": "echo cycle3-{}"}}"#, i)])
            .passes();
    }

    // All 15 items from all cycles should complete
    let all_done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        out.matches("completed").count() >= 15
    });

    if !all_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        let items = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        eprintln!("=== QUEUE STATE ===\n{}\n=== END QUEUE ===", items);
    }

    assert!(all_done, "all 15 items (5 per cycle) should complete after 2 crash cycles");
}
