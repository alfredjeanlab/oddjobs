// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! Storage layer for Odd Jobs

mod migration;
mod snapshot;
mod state;
mod wal;

pub use migration::MigrationError;
pub use snapshot::{Snapshot, SnapshotError, CURRENT_SNAPSHOT_VERSION};
pub use state::{
    CronRecord, MaterializedState, QueueItem, QueueItemStatus, Session, WorkerRecord, Workspace,
    WorkspaceType,
};
pub use wal::{Wal, WalEntry, WalError};
