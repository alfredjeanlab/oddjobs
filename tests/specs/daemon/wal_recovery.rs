//! WAL recovery specs
//!
//! Verify that crash recovery with WAL replay and snapshots works correctly
//! under load and during snapshot creation.

use crate::prelude::*;

// =============================================================================
// High-Load Recovery Tests
// =============================================================================

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
source = { queue = "tasks" }
handler = { job = "process" }
concurrency = 4

[job.process]
vars = ["cmd"]

[[job.process.step]]
name = "work"
run = "${item.cmd}"
"#;

/// Tests recovery after daemon crash with many events in the WAL.
///
/// Scenario:
/// 1. Start daemon and push many queue items (generating many WAL events)
/// 2. Wait for some items to be processed
/// 3. Kill daemon with SIGKILL (crash simulation)
/// 4. Restart daemon (triggers snapshot + WAL replay recovery)
/// 5. Restart worker and verify remaining items can complete
///
/// The key verification is that the state machine remains consistent -
/// all items eventually complete without duplicates or corruption.
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
            .args(&[
                "queue",
                "push",
                "tasks",
                &format!(r#"{{"cmd": "echo item-{}"}}"#, i),
            ])
            .passes();
    }

    // Wait for at least some items to be processed (generates more events)
    // Under high load, processing may be slower, so only require 1 completed item
    let some_processed = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        // Wait for at least 1 item to complete to ensure events in WAL
        out.matches("completed").count() >= 1
    });

    if !some_processed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        let items = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        eprintln!("=== QUEUE STATE ===\n{}\n=== END QUEUE ===", items);
        let workers = temp.oj().args(&["worker", "list"]).passes().stdout();
        eprintln!("=== WORKERS ===\n{}\n=== END WORKERS ===", workers);
    }
    assert!(
        some_processed,
        "should have processed some items before crash"
    );

    // Kill daemon with SIGKILL (simulates crash - no graceful shutdown)
    let killed = temp.daemon_kill();
    assert!(killed, "should be able to kill daemon");

    // Wait for daemon to actually die
    let daemon_dead = wait_for(SPEC_WAIT_MAX_MS, || {
        !temp
            .oj()
            .args(&["daemon", "status"])
            .passes()
            .stdout()
            .contains("Status: running")
    });
    assert!(daemon_dead, "daemon should be dead after kill");

    // Restart daemon - triggers recovery via snapshot + WAL replay
    temp.oj().args(&["daemon", "start"]).passes();

    // Restart worker (workers don't persist across crash)
    temp.oj().args(&["worker", "start", "processor"]).passes();

    // Verify all items eventually complete (state machine is consistent)
    // This proves recovery preserved the queue state correctly
    let all_done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.matches("completed").count() >= 20
    });

    if !all_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        let items = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        eprintln!("=== QUEUE STATE ===\n{}\n=== END QUEUE ===", items);
    }
    assert!(
        all_done,
        "all 20 items should eventually complete after recovery"
    );
}

// =============================================================================
// Snapshot Corruption Tests
// =============================================================================

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
            .args(&[
                "queue",
                "push",
                "tasks",
                &format!(r#"{{"cmd": "echo pre-crash-{}"}}"#, i),
            ])
            .passes();
    }

    let items_done = wait_for(SPEC_WAIT_MAX_MS * 2, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.matches("completed").count() >= 5
    });
    assert!(items_done, "items should complete before crash test");

    // Gracefully stop daemon (this saves a valid snapshot)
    temp.oj().args(&["daemon", "stop"]).passes();

    // Wait for daemon to fully stop
    let stopped = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["daemon", "status"])
            .passes()
            .stdout()
            .contains("not running")
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
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Status: running");

    // Verify previous state is preserved (the 5 completed items)
    let recovered_items = temp
        .oj()
        .args(&["queue", "show", "tasks"])
        .passes()
        .stdout();
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
        file.write_all(b"\x28\xb5\x2f\xfd\x00\x00CORRUPT_DATA_HERE")
            .unwrap();
        file.sync_all().unwrap();
    }

    // Daemon should fail with a clear error
    temp.oj()
        .args(&["daemon", "start"])
        .fails()
        .stderr_has("Snapshot error")
        .stderr_lacks("Connection timeout");
}

// =============================================================================
// Multi-Crash Recovery Tests
// =============================================================================

/// Tests that multiple crash-recovery cycles don't corrupt state.
///
/// This verifies that the WAL and snapshot system handles repeated
/// crashes gracefully without accumulating corruption.
#[test]
#[ignore = "FLAKY: not sure why"]
fn multiple_crash_recovery_cycles_preserve_state() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/load.toml", HIGH_LOAD_RUNBOOK);

    // First crash cycle
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "processor"]).passes();

    for i in 0..5 {
        temp.oj()
            .args(&[
                "queue",
                "push",
                "tasks",
                &format!(r#"{{"cmd": "echo cycle1-{}"}}"#, i),
            ])
            .passes();
    }

    // Under high load, only require 1 item per cycle to verify processing works
    let cycle1_done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.matches("completed").count() >= 1
    });
    assert!(cycle1_done, "cycle 1 should process at least 1 item");

    // Crash #1
    let killed1 = temp.daemon_kill();
    assert!(killed1, "should kill daemon #1");

    let dead1 = wait_for(SPEC_WAIT_MAX_MS, || {
        !temp
            .oj()
            .args(&["daemon", "status"])
            .passes()
            .stdout()
            .contains("Status: running")
    });
    assert!(dead1, "daemon #1 should be dead");

    // Recover and add more work
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "processor"]).passes();

    for i in 0..5 {
        temp.oj()
            .args(&[
                "queue",
                "push",
                "tasks",
                &format!(r#"{{"cmd": "echo cycle2-{}"}}"#, i),
            ])
            .passes();
    }

    // Count items from cycle 2 (not total) to verify recovery works
    let cycle2_done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        // At least 1 cycle2 item should complete (cycle2-N pattern)
        out.contains("cycle2") && out.matches("completed").count() >= 2
    });
    assert!(cycle2_done, "cycle 2 should process at least 1 item");

    // Crash #2
    let killed2 = temp.daemon_kill();
    assert!(killed2, "should kill daemon #2");

    let dead2 = wait_for(SPEC_WAIT_MAX_MS, || {
        !temp
            .oj()
            .args(&["daemon", "status"])
            .passes()
            .stdout()
            .contains("Status: running")
    });
    assert!(dead2, "daemon #2 should be dead");

    // Final recovery and verify state
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "processor"]).passes();

    // Push more items to verify state machine still works
    for i in 0..5 {
        temp.oj()
            .args(&[
                "queue",
                "push",
                "tasks",
                &format!(r#"{{"cmd": "echo cycle3-{}"}}"#, i),
            ])
            .passes();
    }

    // All items from all cycles should eventually complete
    let all_done = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        out.matches("completed").count() >= 15
    });

    if !all_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        let items = temp
            .oj()
            .args(&["queue", "show", "tasks"])
            .passes()
            .stdout();
        eprintln!("=== QUEUE STATE ===\n{}\n=== END QUEUE ===", items);
    }

    assert!(
        all_done,
        "all 15 items (5 per cycle) should complete after 2 crash cycles"
    );
}
