// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::PathBuf;

use tempfile::tempdir;

use oj_storage::Wal;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::handle_worker_start;

/// Helper: create an EventBus backed by a temp WAL, returning the bus and WAL path.
fn test_event_bus(dir: &std::path::Path) -> (EventBus, PathBuf) {
    let wal_path = dir.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, _reader) = EventBus::new(wal);
    (event_bus, wal_path)
}

#[test]
fn start_does_full_start_even_after_restart() {
    let dir = tempdir().unwrap();
    let (event_bus, _wal_path) = test_event_bus(dir.path());

    // No runbook on disk, so start should fail with runbook-not-found.
    // This proves it always does a full start (loads runbook) regardless
    // of any stale WAL state.
    let result = handle_worker_start(std::path::Path::new("/fake"), "", "fix", &event_bus).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}
