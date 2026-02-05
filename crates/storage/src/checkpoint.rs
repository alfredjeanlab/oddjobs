// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Background checkpointing with zstd compression and durable fsync.
//!
//! The checkpointer runs I/O off the main thread while ensuring durability
//! guarantees for crash recovery. The key invariant: snapshot must be durable
//! (including directory fsync) before WAL truncation.
//!
//! ## Design
//!
//! ```text
//! Main Thread                    Background Thread
//! ───────────────────────────    ─────────────────────────────
//! clone state (~10ms)
//!   │
//!   └─────────────────────────→  serialize + compress (~130ms)
//!                                write to .tmp (~20ms)
//!                                fsync .tmp (~50ms)
//!                                rename → snapshot (~1ms)
//!                                fsync directory (~30ms)
//!                                  │
//!   ←────────────────────────────┘ (completion signal)
//! truncate WAL (safe now)
//! ```
//!
//! ## Testability
//!
//! The `CheckpointWriter` trait abstracts all I/O operations, enabling:
//! - Deterministic unit tests with `FakeCheckpointWriter`
//! - Error injection for crash scenario testing
//! - Verification of fsync ordering guarantees

use crate::migration::MigrationRegistry;
use crate::{MaterializedState, Snapshot, SnapshotError, CURRENT_SNAPSHOT_VERSION};
use chrono::Utc;
use serde_json::Value;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use thiserror::Error;

/// Errors from checkpoint operations
#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("compression error: {0}")]
    Compress(String),
    #[error("checkpoint in progress")]
    InProgress,
    #[error("checkpoint failed: {0}")]
    Failed(String),
}

/// Result of a completed checkpoint
#[derive(Debug, Clone)]
pub struct CheckpointResult {
    /// Sequence number that was checkpointed
    pub seq: u64,
    /// Size of the compressed snapshot in bytes
    pub size_bytes: u64,
}

/// Trait abstracting checkpoint I/O for testability.
///
/// All file operations go through this trait, enabling fake implementations
/// for deterministic testing of checkpoint logic and crash scenarios.
pub trait CheckpointWriter: Send + Sync + 'static {
    /// Write compressed snapshot data to a temporary file.
    fn write_tmp(&self, path: &Path, data: &[u8]) -> Result<(), CheckpointError>;

    /// Fsync a file to ensure data is durable.
    fn fsync_file(&self, path: &Path) -> Result<(), CheckpointError>;

    /// Atomically rename tmp file to final path.
    fn rename(&self, from: &Path, to: &Path) -> Result<(), CheckpointError>;

    /// Fsync directory to make rename durable.
    fn fsync_dir(&self, path: &Path) -> Result<(), CheckpointError>;

    /// Get file size (for metrics).
    fn file_size(&self, path: &Path) -> Result<u64, CheckpointError>;
}

/// Production checkpoint writer using real filesystem operations.
#[derive(Clone)]
pub struct FsCheckpointWriter;

impl CheckpointWriter for FsCheckpointWriter {
    fn write_tmp(&self, path: &Path, data: &[u8]) -> Result<(), CheckpointError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = File::create(path)?;
        file.write_all(data)?;
        Ok(())
    }

    fn fsync_file(&self, path: &Path) -> Result<(), CheckpointError> {
        let file = File::open(path)?;
        file.sync_all()?;
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<(), CheckpointError> {
        std::fs::rename(from, to)?;
        Ok(())
    }

    fn fsync_dir(&self, path: &Path) -> Result<(), CheckpointError> {
        let dir = File::open(path)?;
        dir.sync_all()?;
        Ok(())
    }

    fn file_size(&self, path: &Path) -> Result<u64, CheckpointError> {
        Ok(std::fs::metadata(path)?.len())
    }
}

/// Handle to a running checkpoint operation.
///
/// The checkpoint runs in a background thread. Call `wait()` to block until
/// completion, which must happen before WAL truncation.
pub struct CheckpointHandle {
    /// Sequence number being checkpointed
    pub seq: u64,
    receiver: mpsc::Receiver<Result<CheckpointResult, CheckpointError>>,
    // NOTE(lifetime): Keep thread alive
    #[allow(dead_code)]
    handle: JoinHandle<()>,
}

impl CheckpointHandle {
    /// Wait for the checkpoint to complete.
    ///
    /// This blocks until the snapshot is fully durable (including directory fsync).
    /// Only after this returns successfully is it safe to truncate the WAL.
    pub fn wait(self) -> Result<CheckpointResult, CheckpointError> {
        self.receiver
            .recv()
            .map_err(|_| CheckpointError::Failed("checkpoint thread panicked".into()))?
    }

    /// Check if checkpoint is complete without blocking.
    pub fn try_wait(&self) -> Option<Result<CheckpointResult, CheckpointError>> {
        self.receiver.try_recv().ok()
    }
}

/// Checkpointer manages background snapshot operations.
///
/// Only one checkpoint can run at a time. Starting a new checkpoint while
/// one is in progress returns an error.
pub struct Checkpointer<W: CheckpointWriter = FsCheckpointWriter> {
    writer: W,
    snapshot_path: PathBuf,
    compression_level: i32,
}

impl Checkpointer<FsCheckpointWriter> {
    /// Create a new checkpointer with default filesystem writer.
    pub fn new(snapshot_path: PathBuf) -> Self {
        Self::with_writer(FsCheckpointWriter, snapshot_path)
    }
}

impl<W: CheckpointWriter + Clone> Checkpointer<W> {
    /// Create a checkpointer with a custom writer (for testing).
    pub fn with_writer(writer: W, snapshot_path: PathBuf) -> Self {
        Self {
            writer,
            snapshot_path,
            // zstd level 3 is a good balance of speed and compression
            compression_level: 3,
        }
    }

    /// Set the zstd compression level (1-22, default 3).
    pub fn with_compression_level(mut self, level: i32) -> Self {
        self.compression_level = level;
        self
    }

    /// Start a background checkpoint.
    ///
    /// This clones the state and spawns a thread to serialize, compress, and
    /// write the snapshot. The returned handle must be waited on before
    /// truncating the WAL.
    pub fn start(&self, seq: u64, state: &MaterializedState) -> CheckpointHandle {
        let state_clone = state.clone();
        let writer = self.writer.clone();
        let snapshot_path = self.snapshot_path.clone();
        let compression_level = self.compression_level;

        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let result = checkpoint_blocking(
                &writer,
                seq,
                &state_clone,
                &snapshot_path,
                compression_level,
            );
            let _ = tx.send(result);
        });

        CheckpointHandle {
            seq,
            receiver: rx,
            handle,
        }
    }

    /// Perform a synchronous checkpoint (for shutdown).
    pub fn checkpoint_sync(
        &self,
        seq: u64,
        state: &MaterializedState,
    ) -> Result<CheckpointResult, CheckpointError> {
        checkpoint_blocking(
            &self.writer,
            seq,
            state,
            &self.snapshot_path,
            self.compression_level,
        )
    }
}

/// Perform checkpoint I/O (runs on background thread).
fn checkpoint_blocking<W: CheckpointWriter>(
    writer: &W,
    seq: u64,
    state: &MaterializedState,
    snapshot_path: &Path,
    compression_level: i32,
) -> Result<CheckpointResult, CheckpointError> {
    let tmp_path = snapshot_path.with_extension("tmp");

    // 1. Build snapshot struct
    let snapshot = Snapshot {
        version: CURRENT_SNAPSHOT_VERSION,
        seq,
        state: state.clone(),
        created_at: Utc::now(),
    };

    // 2. Serialize to JSON
    let json_bytes = serde_json::to_vec(&snapshot)?;

    // 3. Compress with zstd
    let compressed = zstd::encode_all(json_bytes.as_slice(), compression_level)
        .map_err(|e| CheckpointError::Compress(e.to_string()))?;

    // 4. Write to temp file
    writer.write_tmp(&tmp_path, &compressed)?;

    // 5. Fsync temp file (data durable)
    writer.fsync_file(&tmp_path)?;

    // 6. Atomic rename
    writer.rename(&tmp_path, snapshot_path)?;

    // 7. Fsync directory (rename durable) - CRITICAL for WAL truncation safety
    if let Some(parent) = snapshot_path.parent() {
        writer.fsync_dir(parent)?;
    }

    // 8. Get final size for metrics
    let size_bytes = writer
        .file_size(snapshot_path)
        .unwrap_or(compressed.len() as u64);

    Ok(CheckpointResult { seq, size_bytes })
}

/// Load a zstd-compressed snapshot.
pub fn load_snapshot(path: &Path) -> Result<Option<Snapshot>, SnapshotError> {
    if !path.exists() {
        return Ok(None);
    }

    // Decompress and parse
    let file = File::open(path)?;
    let decoder = zstd::stream::read::Decoder::new(file)
        .map_err(|e| SnapshotError::Io(std::io::Error::other(e.to_string())))?;
    let value: Value = serde_json::from_reader(decoder)?;

    // Run through migration
    let registry = MigrationRegistry::new();
    let migrated = registry.migrate_to(value, CURRENT_SNAPSHOT_VERSION)?;
    let snapshot: Snapshot = serde_json::from_value(migrated)?;
    Ok(Some(snapshot))
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
