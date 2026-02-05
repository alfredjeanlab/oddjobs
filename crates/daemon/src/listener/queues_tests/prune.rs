// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::Event;
use oj_storage::MaterializedState;

use crate::protocol::Response;

use super::super::handle_queue_prune;
use super::{drain_events, project_with_queue_only, push_and_mark_failed, test_event_bus};

/// Helper: push an item and mark it as Completed.
fn push_and_mark_completed(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    data: &[(&str, &str)],
    pushed_at_epoch_ms: u64,
) {
    let data_map = data
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: data_map,
        pushed_at_epoch_ms,
        namespace: namespace.to_string(),
    });
    state.lock().apply_event(&Event::QueueCompleted {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: namespace.to_string(),
    });
}

/// Helper: push an item and mark it as Dead, with a specific pushed_at timestamp.
fn push_and_mark_dead_at(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    data: &[(&str, &str)],
    pushed_at_epoch_ms: u64,
) {
    let data_map = data
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: data_map,
        pushed_at_epoch_ms,
        namespace: namespace.to_string(),
    });
    state.lock().apply_event(&Event::QueueItemDead {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: namespace.to_string(),
    });
}

/// Old timestamp: 24 hours ago (well past the 12h threshold).
fn old_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 24 * 60 * 60 * 1000
}

/// Recent timestamp: 1 hour ago (within the 12h threshold).
fn recent_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 1 * 60 * 60 * 1000
}

#[test]
fn prune_completed_items_older_than_12h() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_completed(
        &state,
        "",
        "tasks",
        "old-item-1",
        &[("task", "a")],
        old_epoch_ms(),
    );

    let result = handle_queue_prune(
        project.path(),
        "",
        "tasks",
        false,
        false,
        &event_bus,
        &state,
    )
    .unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].item_id, "old-item-1");
            assert_eq!(pruned[0].status, "completed");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. } if item_id == "old-item-1"
    ));
}

#[test]
fn prune_skips_recent_completed_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_completed(
        &state,
        "",
        "tasks",
        "recent-item-1",
        &[("task", "b")],
        recent_epoch_ms(),
    );

    let result = handle_queue_prune(
        project.path(),
        "",
        "tasks",
        false,
        false,
        &event_bus,
        &state,
    )
    .unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert!(pruned.is_empty(), "recent items should not be pruned");
            assert_eq!(skipped, 1);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty(), "no events should be emitted");
}

#[test]
fn prune_all_flag_prunes_recent_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_completed(
        &state,
        "",
        "tasks",
        "recent-item-1",
        &[("task", "c")],
        recent_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "tasks", true, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].item_id, "recent-item-1");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
}

#[test]
fn prune_dead_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_dead_at(
        &state,
        "",
        "tasks",
        "dead-item-1",
        &[("task", "d")],
        old_epoch_ms(),
    );

    let result = handle_queue_prune(
        project.path(),
        "",
        "tasks",
        false,
        false,
        &event_bus,
        &state,
    )
    .unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].item_id, "dead-item-1");
            assert_eq!(pruned[0].status, "dead");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. } if item_id == "dead-item-1"
    ));
}

#[test]
fn prune_skips_active_pending_failed_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // Pending item
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "p".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: old_epoch_ms(),
        namespace: String::new(),
    });

    // Active item
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "active-1".to_string(),
        data: [("task".to_string(), "a".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: old_epoch_ms(),
        namespace: String::new(),
    });
    state.lock().apply_event(&Event::QueueTaken {
        queue_name: "tasks".to_string(),
        item_id: "active-1".to_string(),
        worker_name: "w1".to_string(),
        namespace: String::new(),
    });

    // Failed item
    push_and_mark_failed(
        &state,
        "",
        "tasks",
        "failed-1",
        &[("task", "f")],
        old_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "tasks", true, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert!(pruned.is_empty(), "no terminal items to prune");
            assert_eq!(skipped, 3, "pending, active, and failed should be skipped");
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty(), "no events should be emitted");
}

#[test]
fn prune_dry_run_does_not_emit_events() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_completed(
        &state,
        "",
        "tasks",
        "old-item-1",
        &[("task", "a")],
        old_epoch_ms(),
    );

    let result =
        handle_queue_prune(project.path(), "", "tasks", false, true, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert_eq!(pruned.len(), 1, "should report items that would be pruned");
            assert_eq!(pruned[0].item_id, "old-item-1");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(
        events.is_empty(),
        "dry-run should not emit any events, got {:?}",
        events
    );
}

#[test]
fn prune_empty_queue_returns_empty() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result =
        handle_queue_prune(project.path(), "", "tasks", true, false, &event_bus, &state).unwrap();

    match result {
        Response::QueuesPruned {
            ref pruned,
            skipped,
        } => {
            assert!(pruned.is_empty());
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty());
}
