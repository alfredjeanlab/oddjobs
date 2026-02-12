// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Snapshot persistence for crash recovery.
//!
//! Snapshots store the complete materialized state at a point in time,
//! identified by the WAL sequence number. Recovery loads the snapshot
//! and replays WAL entries after that sequence.

use crate::storage::migration::MigrationError;
use crate::storage::MaterializedState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Current snapshot schema version
pub const CURRENT_SNAPSHOT_VERSION: u32 = 1;

/// Errors that can occur in snapshot operations
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("migration error: {0}")]
    Migration(#[from] MigrationError),
}

/// A snapshot of the materialized state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Schema version for migrations
    #[serde(rename = "v")]
    pub version: u32,
    /// WAL sequence number at the time of snapshot
    pub seq: u64,
    /// The complete materialized state
    pub state: MaterializedState,
    /// When this snapshot was created
    pub created_at: DateTime<Utc>,
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
