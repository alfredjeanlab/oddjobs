// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! Storage layer for Odd Jobs

mod snapshot;
mod state;
mod wal;

pub use snapshot::{Snapshot, SnapshotError};
pub use state::{
    MaterializedState, QueueItem, QueueItemStatus, Session, WorkerRecord, Workspace, WorkspaceMode,
};
pub use wal::{Wal, WalEntry, WalError};
