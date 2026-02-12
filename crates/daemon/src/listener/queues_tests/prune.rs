// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::Event;
use oj_storage::MaterializedState;

use crate::protocol::Response;

use super::super::handle_queue_prune;
use super::{
    drain_events, make_ctx, project_with_queue_only, project_without_queue, push_and_mark_failed,
    test_event_bus,
};

/// Helper: push an item and mark it as Completed.
fn push_and_mark_completed(
    state: &Arc<Mutex<MaterializedState>>,
    project: &str,
    queue_name: &str,
    item_id: &str,
    data: &[(&str, &str)],
    pushed_at_ms: u64,
) {
    let data_map = data.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    state.lock().apply_event(&Event::QueuePushed {
        queue: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: data_map,
        pushed_at_ms,
        project: project.to_string(),
    });
    state.lock().apply_event(&Event::QueueCompleted {
        queue: queue_name.to_string(),
        item_id: item_id.to_string(),
        project: project.to_string(),
    });
}

/// Helper: push an item and mark it as Dead, with a specific pushed_at timestamp.
fn push_and_mark_dead_at(
    state: &Arc<Mutex<MaterializedState>>,
    project: &str,
    queue_name: &str,
    item_id: &str,
    data: &[(&str, &str)],
    pushed_at_ms: u64,
) {
    let data_map = data.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    state.lock().apply_event(&Event::QueuePushed {
        queue: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: data_map,
        pushed_at_ms,
        project: project.to_string(),
    });
    state.lock().apply_event(&Event::QueueDead {
        queue: queue_name.to_string(),
        item_id: item_id.to_string(),
        project: project.to_string(),
    });
}

/// Old timestamp: 24 hours ago (well past the 12h threshold).
fn old_epoch_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
        - 24 * 60 * 60 * 1000
}

/// Recent timestamp: 1 hour ago (within the 12h threshold).
fn recent_epoch_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
        - 60 * 60 * 1000
}

#[test]
fn prune_completed_items_older_than_12h() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    push_and_mark_completed(
        &ctx.state,
        "",
        "tasks",
        "old-item-1",
        &[("task", "a")],
        old_epoch_ms(),
    );

    let result = handle_queue_prune(&ctx, project.path(), "", "tasks", false, false).unwrap();

    match result {
        Response::QueuesPruned { ref pruned, skipped } => {
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
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    push_and_mark_completed(
        &ctx.state,
        "",
        "tasks",
        "recent-item-1",
        &[("task", "b")],
        recent_epoch_ms(),
    );

    let result = handle_queue_prune(&ctx, project.path(), "", "tasks", false, false).unwrap();

    match result {
        Response::QueuesPruned { ref pruned, skipped } => {
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
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    push_and_mark_completed(
        &ctx.state,
        "",
        "tasks",
        "recent-item-1",
        &[("task", "c")],
        recent_epoch_ms(),
    );

    let result = handle_queue_prune(&ctx, project.path(), "", "tasks", true, false).unwrap();

    match result {
        Response::QueuesPruned { ref pruned, skipped } => {
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
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    push_and_mark_dead_at(&ctx.state, "", "tasks", "dead-item-1", &[("task", "d")], old_epoch_ms());

    let result = handle_queue_prune(&ctx, project.path(), "", "tasks", false, false).unwrap();

    match result {
        Response::QueuesPruned { ref pruned, skipped } => {
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
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    // Pending item
    ctx.state.lock().apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "p".to_string())].into_iter().collect(),
        pushed_at_ms: old_epoch_ms(),
        project: String::new(),
    });

    // Active item
    ctx.state.lock().apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "active-1".to_string(),
        data: [("task".to_string(), "a".to_string())].into_iter().collect(),
        pushed_at_ms: old_epoch_ms(),
        project: String::new(),
    });
    ctx.state.lock().apply_event(&Event::QueueTaken {
        queue: "tasks".to_string(),
        item_id: "active-1".to_string(),
        worker: "w1".to_string(),
        project: String::new(),
    });

    // Failed item
    push_and_mark_failed(&ctx.state, "", "tasks", "failed-1", &[("task", "f")], old_epoch_ms());

    let result = handle_queue_prune(&ctx, project.path(), "", "tasks", true, false).unwrap();

    match result {
        Response::QueuesPruned { ref pruned, skipped } => {
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
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    push_and_mark_completed(
        &ctx.state,
        "",
        "tasks",
        "old-item-1",
        &[("task", "a")],
        old_epoch_ms(),
    );

    let result = handle_queue_prune(&ctx, project.path(), "", "tasks", false, true).unwrap();

    match result {
        Response::QueuesPruned { ref pruned, skipped } => {
            assert_eq!(pruned.len(), 1, "should report items that would be pruned");
            assert_eq!(pruned[0].item_id, "old-item-1");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty(), "dry-run should not emit any events, got {:?}", events);
}

#[test]
fn prune_empty_queue_returns_empty() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let result = handle_queue_prune(&ctx, project.path(), "", "tasks", true, false).unwrap();

    match result {
        Response::QueuesPruned { ref pruned, skipped } => {
            assert!(pruned.is_empty());
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty());
}

// ── No-runbook fallback tests ─────────────────────────────────────────

#[test]
fn prune_works_without_runbook_definition() {
    let project = project_without_queue();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    // Push and complete an item (with old timestamp so it passes the 12h threshold)
    let data_map = [("task".to_string(), "a".to_string())].into_iter().collect();
    ctx.state.lock().apply_event(&Event::QueuePushed {
        queue: "removed-queue".to_string(),
        item_id: "old-item-1".to_string(),
        data: data_map,
        pushed_at_ms: old_epoch_ms(),
        project: String::new(),
    });
    ctx.state.lock().apply_event(&Event::QueueCompleted {
        queue: "removed-queue".to_string(),
        item_id: "old-item-1".to_string(),
        project: String::new(),
    });

    let result =
        handle_queue_prune(&ctx, project.path(), "", "removed-queue", false, false).unwrap();

    match result {
        Response::QueuesPruned { ref pruned, skipped } => {
            assert_eq!(pruned.len(), 1);
            assert_eq!(pruned[0].item_id, "old-item-1");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected QueuesPruned for queue without runbook, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. } if item_id == "old-item-1"
    ));
}
