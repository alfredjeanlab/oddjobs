// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Snapshot persistence for crash recovery.
//!
//! Snapshots store the complete materialized state at a point in time,
//! identified by the WAL sequence number. Recovery loads the snapshot
//! and replays WAL entries after that sequence.

use crate::MaterializedState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::warn;

/// Errors that can occur in snapshot operations
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// A snapshot of the materialized state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// WAL sequence number at the time of snapshot
    pub seq: u64,
    /// The complete materialized state
    pub state: MaterializedState,
    /// When this snapshot was created
    pub created_at: DateTime<Utc>,
}

impl Snapshot {
    /// Create a new snapshot.
    pub fn new(seq: u64, state: MaterializedState) -> Self {
        Self {
            seq,
            state,
            created_at: Utc::now(),
        }
    }

    /// Save snapshot atomically (write to .tmp, then rename).
    ///
    /// This ensures that a crash during save won't corrupt the snapshot file.
    pub fn save(&self, path: &Path) -> Result<(), SnapshotError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp_path = path.with_extension("tmp");

        // Write to temp file and sync
        {
            let file = File::create(&tmp_path)?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer(&mut writer, self)?;
            let file = writer.into_inner().map_err(|e| e.into_error())?;
            file.sync_all()?;
        }

        // Atomic rename
        fs::rename(&tmp_path, path)?;

        Ok(())
    }

    /// Load snapshot if it exists.
    ///
    /// Returns `Ok(None)` if the file doesn't exist or is corrupt.
    /// Corrupt snapshots are moved to a `.bak` file so the daemon can
    /// recover via WAL replay.
    pub fn load(path: &Path) -> Result<Option<Self>, SnapshotError> {
        if !path.exists() {
            return Ok(None);
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        match serde_json::from_reader(reader) {
            Ok(snapshot) => Ok(Some(snapshot)),
            Err(e) => {
                let bak_path = rotate_bak_path(path);
                warn!(
                    error = %e,
                    path = %path.display(),
                    bak = %bak_path.display(),
                    "Corrupt snapshot, moving to .bak and starting fresh",
                );
                fs::rename(path, &bak_path)?;
                Ok(None)
            }
        }
    }
}

const MAX_BAK_FILES: u32 = 3;

/// Pick the next `.bak` / `.bak.N` path, rotating older backups out.
///
/// Keeps up to [`MAX_BAK_FILES`] backups: `.bak`, `.bak.2`, `.bak.3`.
/// The oldest backup is removed when the limit is reached.
pub(crate) fn rotate_bak_path(path: &Path) -> PathBuf {
    let bak = |n: u32| {
        if n == 1 {
            path.with_extension("bak")
        } else {
            path.with_extension(format!("bak.{n}"))
        }
    };

    // Remove the oldest if at capacity
    let oldest = bak(MAX_BAK_FILES);
    if oldest.exists() {
        let _ = fs::remove_file(&oldest);
    }

    // Shift existing backups up by one
    for n in (1..MAX_BAK_FILES).rev() {
        let src = bak(n);
        if src.exists() {
            let _ = fs::rename(&src, bak(n + 1));
        }
    }

    bak(1)
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
