// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::Event;
use oj_storage::MaterializedState;

use crate::protocol::Response;

use super::super::{handle_queue_drain, handle_queue_drop};
use super::{drain_events, project_with_queue_only, test_event_bus};

// ── Drop tests ────────────────────────────────────────────────────────

#[test]
fn drop_removes_item_from_queue() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a pushed item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "item-abc123".to_string(),
        data: [("task".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_queue_drop(
        project.path(),
        "",
        "tasks",
        "item-abc123",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref queue_name, ref item_id }
            if queue_name == "tasks" && item_id == "item-abc123"
        ),
        "expected QueueDropped, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped {
            queue_name,
            item_id,
            ..
        } if queue_name == "tasks" && item_id == "item-abc123"
    ));
}

#[test]
fn drop_unknown_queue_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_drop(
        project.path(),
        "",
        "nonexistent",
        "item-1",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("nonexistent")),
        "expected Error, got {:?}",
        result
    );
}

#[test]
fn drop_nonexistent_item_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_drop(
        project.path(),
        "",
        "tasks",
        "item-missing",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("not found")),
        "expected Error about item not found, got {:?}",
        result
    );
}

#[test]
fn drop_with_prefix_resolves_unique_match() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "abc12345-0000-0000-0000-000000000000".to_string(),
        data: [("task".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    let result =
        handle_queue_drop(project.path(), "", "tasks", "abc12", &event_bus, &state).unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref item_id, .. }
            if item_id == "abc12345-0000-0000-0000-000000000000"
        ),
        "expected QueueDropped with full ID, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. }
        if item_id == "abc12345-0000-0000-0000-000000000000"
    ));
}

#[test]
fn drop_ambiguous_prefix_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    let mut initial_state = MaterializedState::default();
    for suffix in ["aaa", "bbb"] {
        initial_state.apply_event(&Event::QueuePushed {
            queue_name: "tasks".to_string(),
            item_id: format!("abc-{}", suffix),
            data: [("task".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            pushed_at_epoch_ms: 1_000_000,
            namespace: String::new(),
        });
    }
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_queue_drop(project.path(), "", "tasks", "abc", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("ambiguous")),
        "expected ambiguous error, got {:?}",
        result
    );
}

// ── Drain tests ───────────────────────────────────────────────────────

#[test]
fn drain_removes_all_pending_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with multiple pending items
    let mut initial_state = MaterializedState::default();
    for i in 1..=3 {
        initial_state.apply_event(&Event::QueuePushed {
            queue_name: "tasks".to_string(),
            item_id: format!("item-{}", i),
            data: [("task".to_string(), format!("task-{}", i))]
                .into_iter()
                .collect(),
            pushed_at_epoch_ms: 1_000_000 + i,
            namespace: String::new(),
        });
    }
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_queue_drain(project.path(), "", "tasks", &event_bus, &state).unwrap();

    match result {
        Response::QueueDrained {
            ref queue_name,
            ref items,
        } => {
            assert_eq!(queue_name, "tasks");
            assert_eq!(items.len(), 3);
            let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
            assert!(ids.contains(&"item-1"));
            assert!(ids.contains(&"item-2"));
            assert!(ids.contains(&"item-3"));
        }
        other => panic!("expected QueueDrained, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 3, "expected 3 QueueDropped events");
    for event in &events {
        assert!(
            matches!(event, Event::QueueDropped { queue_name, .. } if queue_name == "tasks"),
            "expected QueueDropped, got {:?}",
            event
        );
    }
}

#[test]
fn drain_skips_non_pending_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    let mut initial_state = MaterializedState::default();
    // One pending item
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "pending".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    // One active item
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "active-1".to_string(),
        data: [("task".to_string(), "active".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 2_000_000,
        namespace: String::new(),
    });
    initial_state.apply_event(&Event::QueueTaken {
        queue_name: "tasks".to_string(),
        item_id: "active-1".to_string(),
        worker_name: "w1".to_string(),
        namespace: String::new(),
    });
    // One dead item
    initial_state.apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "dead-1".to_string(),
        data: [("task".to_string(), "dead".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 3_000_000,
        namespace: String::new(),
    });
    initial_state.apply_event(&Event::QueueItemDead {
        queue_name: "tasks".to_string(),
        item_id: "dead-1".to_string(),
        namespace: String::new(),
    });
    let state = Arc::new(Mutex::new(initial_state));

    let result = handle_queue_drain(project.path(), "", "tasks", &event_bus, &state).unwrap();

    match result {
        Response::QueueDrained { ref items, .. } => {
            assert_eq!(items.len(), 1, "only pending items should be drained");
            assert_eq!(items[0].id, "pending-1");
        }
        other => panic!("expected QueueDrained, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { item_id, .. } if item_id == "pending-1"
    ));
}

#[test]
fn drain_empty_queue_returns_empty_list() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_drain(project.path(), "", "tasks", &event_bus, &state).unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDrained { ref queue_name, ref items }
            if queue_name == "tasks" && items.is_empty()
        ),
        "expected empty QueueDrained, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert!(
        events.is_empty(),
        "no events should be emitted for empty drain"
    );
}

#[test]
fn drain_unknown_queue_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_drain(project.path(), "", "nonexistent", &event_bus, &state).unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("nonexistent")),
        "expected Error, got {:?}",
        result
    );
}

// ── Drop/Drain namespace fallback tests ───────────────────────────────

#[test]
fn drop_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a cron that knows the real project root
    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "my-project/nightly".to_string(),
        oj_storage::CronRecord {
            name: "nightly".to_string(),
            namespace: "my-project".to_string(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            interval: "24h".to_string(),
            run_target: "job:handle".to_string(),
            started_at_ms: 1_000,
            last_fired_at_ms: None,
        },
    );
    // Also add a queue item so the drop has something to find
    initial.apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "item-abc123".to_string(),
        data: [("task".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: "my-project".to_string(),
    });
    let state = Arc::new(Mutex::new(initial));

    let result = handle_queue_drop(
        std::path::Path::new("/wrong/path"),
        "my-project",
        "tasks",
        "item-abc123",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref queue_name, ref item_id }
            if queue_name == "tasks" && item_id == "item-abc123"
        ),
        "expected QueueDropped from namespace fallback, got {:?}",
        result
    );
}

#[test]
fn drain_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a cron that knows the real project root
    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "my-project/nightly".to_string(),
        oj_storage::CronRecord {
            name: "nightly".to_string(),
            namespace: "my-project".to_string(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            interval: "24h".to_string(),
            run_target: "job:handle".to_string(),
            started_at_ms: 1_000,
            last_fired_at_ms: None,
        },
    );
    // Add pending queue items
    initial.apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: "my-project".to_string(),
    });
    let state = Arc::new(Mutex::new(initial));

    let result = handle_queue_drain(
        std::path::Path::new("/wrong/path"),
        "my-project",
        "tasks",
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDrained { ref queue_name, ref items }
            if queue_name == "tasks" && items.len() == 1
        ),
        "expected QueueDrained from namespace fallback, got {:?}",
        result
    );
}
