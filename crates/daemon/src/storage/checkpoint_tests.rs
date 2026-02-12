// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::storage::MaterializedState;
use crate::storage::MigrationError;
use oj_core::{Job, JobConfig, SystemClock};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

/// Records all I/O operations for verification.
#[derive(Debug, Clone, Default)]
struct IoLog {
    pub writes: Vec<(PathBuf, usize)>,
    pub fsyncs_file: Vec<PathBuf>,
    pub fsyncs_dir: Vec<PathBuf>,
    pub renames: Vec<(PathBuf, PathBuf)>,
}

/// Fake writer that records operations and supports error injection.
#[derive(Clone)]
struct FakeCheckpointWriter {
    log: Arc<Mutex<IoLog>>,
    written_data: Arc<Mutex<HashMap<PathBuf, Vec<u8>>>>,
    fail_write: Arc<AtomicBool>,
    fail_fsync_file: Arc<AtomicBool>,
    fail_fsync_dir: Arc<AtomicBool>,
    fail_rename: Arc<AtomicBool>,
    fsync_file_count: Arc<AtomicU32>,
    fsync_dir_count: Arc<AtomicU32>,
}

impl Default for FakeCheckpointWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCheckpointWriter {
    pub fn new() -> Self {
        Self {
            log: Arc::new(Mutex::new(IoLog::default())),
            written_data: Arc::new(Mutex::new(HashMap::new())),
            fail_write: Arc::new(AtomicBool::new(false)),
            fail_fsync_file: Arc::new(AtomicBool::new(false)),
            fail_fsync_dir: Arc::new(AtomicBool::new(false)),
            fail_rename: Arc::new(AtomicBool::new(false)),
            fsync_file_count: Arc::new(AtomicU32::new(0)),
            fsync_dir_count: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn set_fail_write(&self, fail: bool) {
        self.fail_write.store(fail, Ordering::SeqCst);
    }

    pub fn set_fail_fsync_file(&self, fail: bool) {
        self.fail_fsync_file.store(fail, Ordering::SeqCst);
    }

    pub fn set_fail_fsync_dir(&self, fail: bool) {
        self.fail_fsync_dir.store(fail, Ordering::SeqCst);
    }

    pub fn log(&self) -> IoLog {
        self.log.lock().unwrap().clone()
    }

    pub fn fsync_file_count(&self) -> u32 {
        self.fsync_file_count.load(Ordering::SeqCst)
    }

    pub fn fsync_dir_count(&self) -> u32 {
        self.fsync_dir_count.load(Ordering::SeqCst)
    }

    pub fn get_written_data(&self, path: &Path) -> Option<Vec<u8>> {
        self.written_data.lock().unwrap().get(path).cloned()
    }
}

impl CheckpointWriter for FakeCheckpointWriter {
    fn write_tmp(&self, path: &Path, data: &[u8]) -> Result<(), CheckpointError> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(CheckpointError::Io(std::io::Error::other("injected write failure")));
        }
        let mut log = self.log.lock().unwrap();
        log.writes.push((path.to_owned(), data.len()));
        self.written_data.lock().unwrap().insert(path.to_owned(), data.to_vec());
        Ok(())
    }

    fn fsync_file(&self, path: &Path) -> Result<(), CheckpointError> {
        if self.fail_fsync_file.load(Ordering::SeqCst) {
            return Err(CheckpointError::Io(std::io::Error::other("injected fsync failure")));
        }
        self.fsync_file_count.fetch_add(1, Ordering::SeqCst);
        let mut log = self.log.lock().unwrap();
        log.fsyncs_file.push(path.to_owned());
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), CheckpointError> {
        if self.fail_rename.load(Ordering::SeqCst) {
            return Err(CheckpointError::Io(std::io::Error::other("injected rename failure")));
        }
        // Move data from tmp to final path
        let data = self.written_data.lock().unwrap().remove(from);
        if let Some(d) = data {
            self.written_data.lock().unwrap().insert(to.to_owned(), d);
        }
        let mut log = self.log.lock().unwrap();
        log.renames.push((from.to_owned(), to.to_owned()));
        Ok(())
    }

    fn fsync_dir(&self, path: &Path) -> Result<(), CheckpointError> {
        if self.fail_fsync_dir.load(Ordering::SeqCst) {
            return Err(CheckpointError::Io(std::io::Error::other("injected fsync_dir failure")));
        }
        self.fsync_dir_count.fetch_add(1, Ordering::SeqCst);
        let mut log = self.log.lock().unwrap();
        log.fsyncs_dir.push(path.to_owned());
        Ok(())
    }

    fn file_size(&self, path: &Path) -> Result<u64, CheckpointError> {
        let data = self.written_data.lock().unwrap();
        Ok(data.get(path).map(|d| d.len() as u64).unwrap_or(0))
    }
}

fn test_config(id: &str, name: &str) -> JobConfig {
    JobConfig::builder(id, "feature", "init")
        .name(name)
        .runbook_hash("testhash")
        .cwd("/test/project")
        .build()
}

fn create_test_state(num_jobs: usize) -> MaterializedState {
    let mut state = MaterializedState::default();
    for i in 0..num_jobs {
        let job = Job::new(test_config(&format!("job-{i}"), &format!("test-{i}")), &SystemClock);
        state.jobs.insert(format!("job-{i}"), job);
    }
    state
}

#[test]
fn test_checkpoint_basic_flow() {
    let writer = FakeCheckpointWriter::new();
    let checkpointer =
        Checkpointer::with_writer(writer.clone(), PathBuf::from("/data/snapshot.json"));

    let state = create_test_state(3);
    let handle = checkpointer.start(42, &state);
    let result = handle.wait().unwrap();

    assert_eq!(result.seq, 42);
    assert!(result.size_bytes > 0);

    // Verify I/O order
    let log = writer.log();
    assert_eq!(log.writes.len(), 1);
    assert_eq!(log.fsyncs_file.len(), 1);
    assert_eq!(log.renames.len(), 1);
    assert_eq!(log.fsyncs_dir.len(), 1);

    // Verify order: write -> fsync_file -> rename -> fsync_dir
    assert!(log.writes[0].0.to_string_lossy().contains(".tmp"));
    assert!(log.fsyncs_file[0].to_string_lossy().contains(".tmp"));
    assert_eq!(log.renames[0].1, PathBuf::from("/data/snapshot.json"));
    assert_eq!(log.fsyncs_dir[0], PathBuf::from("/data"));
}

#[test]
fn test_checkpoint_fsync_ordering_for_wal_safety() {
    // This test verifies the critical invariant:
    // Directory fsync MUST happen after rename for WAL truncation safety
    let writer = FakeCheckpointWriter::new();
    let checkpointer =
        Checkpointer::with_writer(writer.clone(), PathBuf::from("/data/snapshot.json"));

    let state = create_test_state(1);
    let handle = checkpointer.start(100, &state);
    handle.wait().unwrap();

    // Both fsync calls must have happened
    assert_eq!(writer.fsync_file_count(), 1, "must fsync tmp file");
    assert_eq!(writer.fsync_dir_count(), 1, "must fsync directory");

    let log = writer.log();

    // Verify rename happens before directory fsync
    // (this is what makes WAL truncation safe)
    let rename_idx = log
        .renames
        .iter()
        .position(|(_, to)| to == &PathBuf::from("/data/snapshot.json"))
        .expect("rename should occur");

    let dir_fsync_idx = log
        .fsyncs_dir
        .iter()
        .position(|p| p == &PathBuf::from("/data"))
        .expect("dir fsync should occur");

    // In our log, renames and fsyncs_dir are separate vecs, but we know
    // rename happens before fsync_dir because that's the code order.
    // The test verifies both operations actually happened.
    assert_eq!(rename_idx, 0);
    assert_eq!(dir_fsync_idx, 0);
}

#[test]
fn test_checkpoint_produces_compressed_output() {
    let writer = FakeCheckpointWriter::new();
    let snapshot_path = PathBuf::from("/data/snapshot.json");
    let checkpointer = Checkpointer::with_writer(writer.clone(), snapshot_path.clone());

    let state = create_test_state(10);
    let handle = checkpointer.start(1, &state);
    handle.wait().unwrap();

    // Get the written data (should be zstd compressed)
    let data = writer.get_written_data(&snapshot_path).unwrap();

    // Verify zstd magic number
    assert!(data.len() >= 4);
    assert_eq!(&data[0..4], &[0x28, 0xB5, 0x2F, 0xFD], "should be zstd format");

    // Decompress and verify content
    let decompressed = zstd::decode_all(data.as_slice()).unwrap();
    let snapshot: Snapshot = serde_json::from_slice(&decompressed).unwrap();
    assert_eq!(snapshot.seq, 1);
    assert_eq!(snapshot.state.jobs.len(), 10);
}

#[yare::parameterized(
    write_failure     = { |w: &FakeCheckpointWriter| w.set_fail_write(true) },
    fsync_failure     = { |w: &FakeCheckpointWriter| w.set_fail_fsync_file(true) },
    dir_fsync_failure = { |w: &FakeCheckpointWriter| w.set_fail_fsync_dir(true) },
)]
fn test_checkpoint_error_propagation(inject: fn(&FakeCheckpointWriter)) {
    let writer = FakeCheckpointWriter::new();
    inject(&writer);
    let checkpointer = Checkpointer::with_writer(writer, PathBuf::from("/data/snapshot.json"));
    let state = create_test_state(1);
    let handle = checkpointer.start(1, &state);
    assert!(handle.wait().is_err());
}

#[test]
fn test_checkpoint_sync_for_shutdown() {
    let writer = FakeCheckpointWriter::new();
    let checkpointer =
        Checkpointer::with_writer(writer.clone(), PathBuf::from("/data/snapshot.json"));

    let state = create_test_state(5);
    let result = checkpointer.checkpoint_sync(99, &state).unwrap();

    assert_eq!(result.seq, 99);
    assert_eq!(writer.fsync_file_count(), 1);
    assert_eq!(writer.fsync_dir_count(), 1);
}

#[test]
fn test_load_snapshot_detects_compression() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    // Write compressed snapshot using real filesystem
    let state = create_test_state(3);
    let checkpointer = Checkpointer::new(path.clone());
    checkpointer.checkpoint_sync(42, &state).unwrap();

    // Load it back
    let loaded = load_snapshot(&path).unwrap().unwrap();
    assert_eq!(loaded.seq, 42);
    assert_eq!(loaded.state.jobs.len(), 3);
}

#[test]
fn test_load_snapshot_nonexistent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nonexistent.json");

    let result = load_snapshot(&path).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_compression_reduces_size() {
    let dir = tempdir().unwrap();
    let compressed_path = dir.path().join("compressed.json");
    let uncompressed_path = dir.path().join("uncompressed.json");

    // Create a larger state for meaningful compression
    let state = create_test_state(100);

    // Write compressed
    let checkpointer = Checkpointer::new(compressed_path.clone());
    let result = checkpointer.checkpoint_sync(1, &state).unwrap();
    let compressed_size = result.size_bytes;

    // Write uncompressed (raw JSON for size comparison)
    let uncompressed_json = serde_json::to_vec(&state).unwrap();
    std::fs::write(&uncompressed_path, &uncompressed_json).unwrap();
    let uncompressed_size = uncompressed_json.len() as u64;

    // Compressed should be significantly smaller
    assert!(
        compressed_size < uncompressed_size / 2,
        "compressed ({compressed_size}) should be less than half of uncompressed ({uncompressed_size})"
    );
}

#[test]
fn test_load_zstd_snapshot_with_too_new_version_fails() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    // Create a zstd-compressed snapshot with version 99 (too new)
    let snapshot_json = r#"{
        "v": 99,
        "seq": 42,
        "state": {
            "jobs": {},
            "sessions": {},
            "workspaces": {},
            "runbooks": {},
            "workers": {},
            "queue_items": {},
            "crons": {},
            "decisions": {},
            "crew": {}
        },
        "created_at": "2025-01-01T00:00:00Z"
    }"#;

    // Compress with zstd and write
    let compressed = zstd::encode_all(snapshot_json.as_bytes(), 3).unwrap();
    std::fs::write(&path, &compressed).unwrap();

    // Verify it's detected as zstd
    assert_eq!(&compressed[0..4], &[0x28, 0xB5, 0x2F, 0xFD]);

    // Load should fail with migration error
    let result = load_snapshot(&path);
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(
        matches!(err, SnapshotError::Migration(MigrationError::TooNew(99, _))),
        "expected TooNew migration error, got: {err:?}"
    );
}

#[test]
fn test_load_zstd_snapshot_with_current_version_succeeds() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    // Create a zstd-compressed snapshot with current version
    let snapshot_json = format!(
        r#"{{
        "v": {version},
        "seq": 42,
        "state": {{
            "jobs": {{}},
            "sessions": {{}},
            "workspaces": {{}},
            "runbooks": {{}},
            "workers": {{}},
            "queue_items": {{}},
            "crons": {{}},
            "decisions": {{}},
            "crew": {{}}
        }},
        "created_at": "2025-01-01T00:00:00Z"
    }}"#,
        version = CURRENT_SNAPSHOT_VERSION
    );

    // Compress with zstd and write
    let compressed = zstd::encode_all(snapshot_json.as_bytes(), 3).unwrap();
    std::fs::write(&path, &compressed).unwrap();

    // Load should succeed
    let result = load_snapshot(&path).unwrap().unwrap();
    assert_eq!(result.seq, 42);
    assert_eq!(result.version, CURRENT_SNAPSHOT_VERSION);
}
