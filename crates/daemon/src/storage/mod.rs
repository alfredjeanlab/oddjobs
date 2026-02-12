// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Storage layer for Odd Jobs

mod checkpoint;
mod migration;
mod snapshot;
mod state;
mod wal;

pub use checkpoint::{load_snapshot, Checkpointer};
pub use snapshot::{Snapshot, SnapshotError, CURRENT_SNAPSHOT_VERSION};
pub use state::{CronRecord, MaterializedState, QueueItemStatus, QueuePollMeta, WorkerRecord};
pub use wal::{Wal, WalEntry, WalError};

#[cfg(test)]
pub(crate) use migration::MigrationError;
#[cfg(test)]
pub use state::{QueueItem, Workspace, WorkspaceType};
