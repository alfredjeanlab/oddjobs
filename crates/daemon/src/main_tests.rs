use std::sync::Mutex;
use std::time::Duration;

use super::timer_check_interval;

/// Serialise tests that mutate `OJ_TIMER_CHECK_MS` to avoid env-var races.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn timer_check_interval_default() {
    let _lock = ENV_LOCK.lock().unwrap();
    std::env::remove_var("OJ_TIMER_CHECK_MS");
    assert_eq!(timer_check_interval(), Duration::from_secs(1));
}

#[test]
fn timer_check_interval_from_env() {
    let _lock = ENV_LOCK.lock().unwrap();
    std::env::set_var("OJ_TIMER_CHECK_MS", "500");
    assert_eq!(timer_check_interval(), Duration::from_millis(500));
    std::env::remove_var("OJ_TIMER_CHECK_MS");
}

#[test]
fn timer_check_interval_invalid_env_falls_back_to_default() {
    let _lock = ENV_LOCK.lock().unwrap();
    std::env::set_var("OJ_TIMER_CHECK_MS", "not_a_number");
    assert_eq!(timer_check_interval(), Duration::from_secs(1));
    std::env::remove_var("OJ_TIMER_CHECK_MS");
}

// --- rotate_log_if_needed tests ---

use super::{rotate_log_if_needed, MAX_LOG_SIZE};
use std::io::Write;

fn write_bytes(path: &std::path::Path, size: u64) {
    let mut f = std::fs::File::create(path).unwrap();
    let buf = vec![b'x'; size as usize];
    f.write_all(&buf).unwrap();
}

#[test]
fn rotate_skips_small_file() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("daemon.log");
    write_bytes(&log, 1024);

    rotate_log_if_needed(&log);

    assert!(log.exists(), "small log should not be rotated");
    assert!(!dir.path().join("daemon.log.1").exists());
}

#[test]
fn rotate_moves_large_file() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("daemon.log");
    write_bytes(&log, MAX_LOG_SIZE + 1);

    rotate_log_if_needed(&log);

    assert!(!log.exists(), "original should be renamed");
    assert!(dir.path().join("daemon.log.1").exists());
}

#[test]
fn rotate_shifts_existing_rotations() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("daemon.log");

    // Create existing rotations
    write_bytes(&dir.path().join("daemon.log.1"), 100);
    write_bytes(&dir.path().join("daemon.log.2"), 200);

    // Create oversized current log
    write_bytes(&log, MAX_LOG_SIZE + 1);

    rotate_log_if_needed(&log);

    assert!(!log.exists());
    // .1 is the freshly rotated file
    assert!(dir.path().join("daemon.log.1").exists());
    // old .1 shifted to .2
    assert!(dir.path().join("daemon.log.2").exists());
    // old .2 shifted to .3
    assert!(dir.path().join("daemon.log.3").exists());

    // Verify content shifted correctly: old .2 (200 bytes) → .3
    assert_eq!(std::fs::metadata(dir.path().join("daemon.log.3")).unwrap().len(), 200);
}

#[test]
fn rotate_drops_oldest_when_full() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("daemon.log");

    write_bytes(&dir.path().join("daemon.log.1"), 100);
    write_bytes(&dir.path().join("daemon.log.2"), 200);
    write_bytes(&dir.path().join("daemon.log.3"), 300);

    write_bytes(&log, MAX_LOG_SIZE + 1);

    rotate_log_if_needed(&log);

    assert!(!log.exists());
    assert!(dir.path().join("daemon.log.1").exists());
    assert!(dir.path().join("daemon.log.2").exists());
    assert!(dir.path().join("daemon.log.3").exists());

    // .3 should now be the old .2 (200 bytes), not the old .3 (300 bytes)
    assert_eq!(std::fs::metadata(dir.path().join("daemon.log.3")).unwrap().len(), 200);
}

#[test]
fn rotate_noop_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("daemon.log");

    // Should not panic
    rotate_log_if_needed(&log);
}
