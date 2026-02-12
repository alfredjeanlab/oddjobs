// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::Event;
use oj_storage::MaterializedState;

use crate::protocol::Response;

use super::super::{handle_queue_drain, handle_queue_drop};
use super::{
    drain_events, make_ctx, project_with_queue_only, project_without_queue, test_event_bus,
};

// ── Drop tests ────────────────────────────────────────────────────────

#[test]
fn drop_removes_item_from_queue() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a pushed item
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "item-abc123".to_string(),
        data: [("task".to_string(), "test".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    let result = handle_queue_drop(&ctx, project.path(), "", "tasks", "item-abc123").unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref queue, ref item_id }
            if queue == "tasks" && item_id == "item-abc123"
        ),
        "expected QueueDropped, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped {
            queue,
            item_id,
            ..
        } if queue == "tasks" && item_id == "item-abc123"
    ));
}

#[test]
fn drop_unknown_queue_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let result = handle_queue_drop(&ctx, project.path(), "", "nonexistent", "item-1").unwrap();

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
    let ctx = make_ctx(event_bus, state);

    let result = handle_queue_drop(&ctx, project.path(), "", "tasks", "item-missing").unwrap();

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
        queue: "tasks".to_string(),
        item_id: "abc12345-0000-0000-0000-000000000000".to_string(),
        data: [("task".to_string(), "test".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    let result = handle_queue_drop(&ctx, project.path(), "", "tasks", "abc12").unwrap();

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
            queue: "tasks".to_string(),
            item_id: format!("abc-{}", suffix),
            data: [("task".to_string(), "test".to_string())].into_iter().collect(),
            pushed_at_ms: 1_000_000,
            project: String::new(),
        });
    }
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    let result = handle_queue_drop(&ctx, project.path(), "", "tasks", "abc").unwrap();

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
            queue: "tasks".to_string(),
            item_id: format!("item-{}", i),
            data: [("task".to_string(), format!("task-{}", i))].into_iter().collect(),
            pushed_at_ms: 1_000_000 + i,
            project: String::new(),
        });
    }
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    let result = handle_queue_drain(&ctx, project.path(), "", "tasks").unwrap();

    match result {
        Response::QueueDrained { ref queue, ref items } => {
            assert_eq!(queue, "tasks");
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
            matches!(event, Event::QueueDropped { queue, .. } if queue == "tasks"),
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
        queue: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "pending".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    // One active item
    initial_state.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "active-1".to_string(),
        data: [("task".to_string(), "active".to_string())].into_iter().collect(),
        pushed_at_ms: 2_000_000,
        project: String::new(),
    });
    initial_state.apply_event(&Event::QueueTaken {
        queue: "tasks".to_string(),
        item_id: "active-1".to_string(),
        worker: "w1".to_string(),
        project: String::new(),
    });
    // One dead item
    initial_state.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "dead-1".to_string(),
        data: [("task".to_string(), "dead".to_string())].into_iter().collect(),
        pushed_at_ms: 3_000_000,
        project: String::new(),
    });
    initial_state.apply_event(&Event::QueueDead {
        queue: "tasks".to_string(),
        item_id: "dead-1".to_string(),
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    let result = handle_queue_drain(&ctx, project.path(), "", "tasks").unwrap();

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
    let ctx = make_ctx(event_bus, state);

    let result = handle_queue_drain(&ctx, project.path(), "", "tasks").unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDrained { ref queue, ref items }
            if queue == "tasks" && items.is_empty()
        ),
        "expected empty QueueDrained, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert!(events.is_empty(), "no events should be emitted for empty drain");
}

#[test]
fn drain_unknown_queue_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let result = handle_queue_drain(&ctx, project.path(), "", "nonexistent").unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("nonexistent")),
        "expected Error, got {:?}",
        result
    );
}

// ── Drop/Drain project fallback tests ───────────────────────────────

#[test]
fn drop_with_wrong_project_path_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a cron that knows the real project path
    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "my-project/nightly".to_string(),
        oj_storage::CronRecord {
            name: "nightly".to_string(),
            project: "my-project".to_string(),
            project_path: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            interval: "24h".to_string(),
            target: oj_core::RunTarget::job("handle"),
            started_at_ms: 1_000,
            last_fired_at_ms: None,
        },
    );
    // Also add a queue item so the drop has something to find
    initial.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "item-abc123".to_string(),
        data: [("task".to_string(), "test".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: "my-project".to_string(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial)));

    let result = handle_queue_drop(
        &ctx,
        std::path::Path::new("/wrong/path"),
        "my-project",
        "tasks",
        "item-abc123",
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref queue, ref item_id }
            if queue == "tasks" && item_id == "item-abc123"
        ),
        "expected QueueDropped from project fallback, got {:?}",
        result
    );
}

#[test]
fn drain_with_wrong_project_path_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a cron that knows the real project path
    let mut initial = MaterializedState::default();
    initial.crons.insert(
        "my-project/nightly".to_string(),
        oj_storage::CronRecord {
            name: "nightly".to_string(),
            project: "my-project".to_string(),
            project_path: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "running".to_string(),
            interval: "24h".to_string(),
            target: oj_core::RunTarget::job("handle"),
            started_at_ms: 1_000,
            last_fired_at_ms: None,
        },
    );
    // Add pending queue items
    initial.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "test".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: "my-project".to_string(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial)));

    let result =
        handle_queue_drain(&ctx, std::path::Path::new("/wrong/path"), "my-project", "tasks")
            .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDrained { ref queue, ref items }
            if queue == "tasks" && items.len() == 1
        ),
        "expected QueueDrained from project fallback, got {:?}",
        result
    );
}

// ── No-runbook fallback tests ─────────────────────────────────────────

#[test]
fn drop_works_without_runbook_definition() {
    let project = project_without_queue();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a queue item (persisted but no runbook definition)
    let mut initial_state = MaterializedState::default();
    initial_state.apply_event(&Event::QueuePushed {
        queue: "removed-queue".to_string(),
        item_id: "item-orphan-1".to_string(),
        data: [("task".to_string(), "test".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    let result =
        handle_queue_drop(&ctx, project.path(), "", "removed-queue", "item-orphan-1").unwrap();

    assert!(
        matches!(
            result,
            Response::QueueDropped { ref queue, ref item_id }
            if queue == "removed-queue" && item_id == "item-orphan-1"
        ),
        "expected QueueDropped for queue without runbook, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Event::QueueDropped { queue, item_id, .. }
        if queue == "removed-queue" && item_id == "item-orphan-1"
    ));
}

#[test]
fn drop_unknown_queue_not_in_state_returns_error() {
    let project = project_without_queue();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let result = handle_queue_drop(&ctx, project.path(), "", "totally-unknown", "item-1").unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("totally-unknown")),
        "expected Error for queue not in runbook or state, got {:?}",
        result
    );
}

#[test]
fn drain_works_without_runbook_definition() {
    let project = project_without_queue();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());

    let mut initial_state = MaterializedState::default();
    for i in 1..=2 {
        initial_state.apply_event(&Event::QueuePushed {
            queue: "removed-queue".to_string(),
            item_id: format!("item-{}", i),
            data: [("task".to_string(), format!("task-{}", i))].into_iter().collect(),
            pushed_at_ms: 1_000_000 + i,
            project: String::new(),
        });
    }
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial_state)));

    let result = handle_queue_drain(&ctx, project.path(), "", "removed-queue").unwrap();

    match result {
        Response::QueueDrained { ref queue, ref items } => {
            assert_eq!(queue, "removed-queue");
            assert_eq!(items.len(), 2);
        }
        other => panic!("expected QueueDrained for queue without runbook, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 2);
}
