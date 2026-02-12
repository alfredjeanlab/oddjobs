// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use crate::storage::MaterializedState;
use oj_core::Event;

use crate::protocol::Response;

use super::super::{handle_queue_retry, RetryFilter};
use super::{
    drain_events, make_ctx, project_with_queue_only, project_without_queue, push_and_mark_dead,
    push_and_mark_failed, test_event_bus,
};

// ── Single-item retry tests ───────────────────────────────────────────

#[test]
fn retry_with_prefix_resolves_unique_match() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    push_and_mark_dead(
        &ctx.state,
        "",
        "tasks",
        "def98765-0000-0000-0000-000000000000",
        &[("task", "retry-me")],
    );

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter { item_ids: &["def98".to_string()], all_dead: false, status_filter: None },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref item_ids, .. } => {
            assert_eq!(item_ids, &["def98765-0000-0000-0000-000000000000"]);
        }
        other => panic!("expected QueueRetried with full ID, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::QueueRetry { item_id, .. } if item_id == "def98765-0000-0000-0000-000000000000")));
}

#[test]
fn retry_with_exact_id_still_works() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    push_and_mark_dead(&ctx.state, "", "tasks", "exact-id-1234", &[("task", "retry-me")]);

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter {
            item_ids: &["exact-id-1234".to_string()],
            all_dead: false,
            status_filter: None,
        },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref item_ids, .. } => {
            assert_eq!(item_ids, &["exact-id-1234"]);
        }
        other => panic!("expected QueueRetried, got {:?}", other),
    }
}

#[test]
fn retry_ambiguous_prefix_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    for suffix in ["aaa", "bbb"] {
        push_and_mark_dead(
            &ctx.state,
            "",
            "tasks",
            &format!("abc-{}", suffix),
            &[("task", "test")],
        );
    }

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter { item_ids: &["abc".to_string()], all_dead: false, status_filter: None },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref not_found, ref item_ids, .. } => {
            assert_eq!(not_found, &["abc"]);
            assert!(item_ids.is_empty());
        }
        other => panic!("expected QueueRetried with not_found, got {:?}", other),
    }
}

#[test]
fn retry_no_match_returns_not_found() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter {
            item_ids: &["nonexistent".to_string()],
            all_dead: false,
            status_filter: None,
        },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref not_found, ref item_ids, .. } => {
            assert_eq!(not_found, &["nonexistent"]);
            assert!(item_ids.is_empty());
        }
        other => panic!("expected QueueRetried with not_found, got {:?}", other),
    }
}

// ── Bulk retry tests ──────────────────────────────────────────────────

#[test]
fn retry_bulk_multiple_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    // Create multiple dead items
    for i in 1..=3 {
        push_and_mark_dead(&ctx.state, "", "tasks", &format!("item-{}", i), &[("task", "test")]);
    }

    let ids = ["item-1".to_string(), "item-2".to_string(), "item-3".to_string()];
    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter { item_ids: &ids, all_dead: false, status_filter: None },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref queue, ref item_ids, ref already_retried, ref not_found } => {
            assert_eq!(queue, "tasks");
            assert_eq!(item_ids.len(), 3);
            assert!(item_ids.contains(&"item-1".to_string()));
            assert!(item_ids.contains(&"item-2".to_string()));
            assert!(item_ids.contains(&"item-3".to_string()));
            assert!(already_retried.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected QueueRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 3, "expected 3 QueueRetry events");
    for event in &events {
        assert!(
            matches!(event, Event::QueueRetry { queue, .. } if queue == "tasks"),
            "expected QueueRetry, got {:?}",
            event
        );
    }
}

#[test]
fn retry_all_dead_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    // Create multiple items in different states
    push_and_mark_dead(&ctx.state, "", "tasks", "dead-1", &[("task", "d1")]);
    push_and_mark_dead(&ctx.state, "", "tasks", "dead-2", &[("task", "d2")]);
    // One pending item (should be skipped)
    ctx.state.lock().apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "p1".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter { item_ids: &[], all_dead: true, status_filter: None },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref queue, ref item_ids, ref already_retried, ref not_found } => {
            assert_eq!(queue, "tasks");
            assert_eq!(item_ids.len(), 2, "only dead items should be retried");
            assert!(item_ids.contains(&"dead-1".to_string()));
            assert!(item_ids.contains(&"dead-2".to_string()));
            assert!(already_retried.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected QueueRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 2, "expected 2 QueueRetry events");
}

#[test]
fn retry_by_status_failed() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    // Create items in different states
    push_and_mark_dead(&ctx.state, "", "tasks", "dead-1", &[("task", "d1")]);
    push_and_mark_failed(&ctx.state, "", "tasks", "failed-1", &[("task", "f1")], 1_000_000);
    push_and_mark_failed(&ctx.state, "", "tasks", "failed-2", &[("task", "f2")], 1_000_000);

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter { item_ids: &[], all_dead: false, status_filter: Some("failed") },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref queue, ref item_ids, ref already_retried, ref not_found } => {
            assert_eq!(queue, "tasks");
            assert_eq!(item_ids.len(), 2, "only failed items should be retried");
            assert!(item_ids.contains(&"failed-1".to_string()));
            assert!(item_ids.contains(&"failed-2".to_string()));
            assert!(!item_ids.contains(&"dead-1".to_string()), "dead items should not be included");
            assert!(already_retried.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected QueueRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 2, "expected 2 QueueRetry events");
}

#[test]
fn retry_bulk_mixed_results() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    // Create one dead item (can be retried)
    push_and_mark_dead(&ctx.state, "", "tasks", "dead-1", &[("task", "d1")]);
    // Create one pending item (cannot be retried - not dead/failed)
    ctx.state.lock().apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "p1".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    // "nonexistent" doesn't exist

    let ids = ["dead-1".to_string(), "pending-1".to_string(), "nonexistent".to_string()];
    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter { item_ids: &ids, all_dead: false, status_filter: None },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref queue, ref item_ids, ref already_retried, ref not_found } => {
            assert_eq!(queue, "tasks");
            assert_eq!(item_ids, &["dead-1".to_string()]);
            assert_eq!(already_retried, &["pending-1".to_string()]);
            assert_eq!(not_found, &["nonexistent".to_string()]);
        }
        other => panic!("expected QueueRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1, "only 1 QueueRetry event for dead-1");
}

#[test]
fn retry_all_dead_empty_queue() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, state);

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "tasks",
        RetryFilter { item_ids: &[], all_dead: true, status_filter: None },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref queue, ref item_ids, ref already_retried, ref not_found } => {
            assert_eq!(queue, "tasks");
            assert!(item_ids.is_empty());
            assert!(already_retried.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected empty QueueRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty());
}

// ── Retry project fallback test ─────────────────────────────────────

#[test]
fn retry_with_wrong_project_path_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a worker that knows the real project root
    let mut initial = MaterializedState::default();
    initial.workers.insert(
        "my-project/processor".to_string(),
        crate::storage::WorkerRecord {
            name: "processor".to_string(),
            project_path: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "stopped".to_string(),
            active: vec![],
            queue: "tasks".to_string(),
            concurrency: 1,
            owners: HashMap::new(),
            project: "my-project".to_string(),
        },
    );
    // Apply directly to initial state
    initial.apply_event(&Event::QueuePushed {
        queue: "tasks".to_string(),
        item_id: "item-dead-1".to_string(),
        data: [("task".to_string(), "retry-me".to_string())].into_iter().collect(),
        pushed_at_ms: 1_000_000,
        project: "my-project".to_string(),
    });
    initial.apply_event(&Event::QueueDead {
        queue: "tasks".to_string(),
        item_id: "item-dead-1".to_string(),
        project: "my-project".to_string(),
    });
    let ctx = make_ctx(event_bus, Arc::new(Mutex::new(initial)));

    let result = handle_queue_retry(
        &ctx,
        std::path::Path::new("/wrong/path"),
        "my-project",
        "tasks",
        RetryFilter {
            item_ids: &["item-dead-1".to_string()],
            all_dead: false,
            status_filter: None,
        },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref queue, ref item_ids, .. } => {
            assert_eq!(queue, "tasks");
            assert_eq!(item_ids, &["item-dead-1"]);
        }
        other => panic!("expected QueueRetried from project fallback, got {:?}", other),
    }
}

// ── No-runbook fallback tests ─────────────────────────────────────────

#[test]
fn retry_works_without_runbook_definition() {
    let project = project_without_queue();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    // Pre-populate state with a dead item in a queue that has no runbook definition
    let data_map = [("task".to_string(), "orphan".to_string())].into_iter().collect();
    ctx.state.lock().apply_event(&Event::QueuePushed {
        queue: "removed-queue".to_string(),
        item_id: "dead-orphan-1".to_string(),
        data: data_map,
        pushed_at_ms: 1_000_000,
        project: String::new(),
    });
    ctx.state.lock().apply_event(&Event::QueueDead {
        queue: "removed-queue".to_string(),
        item_id: "dead-orphan-1".to_string(),
        project: String::new(),
    });

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "removed-queue",
        RetryFilter {
            item_ids: &["dead-orphan-1".to_string()],
            all_dead: false,
            status_filter: None,
        },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref queue, ref item_ids, .. } => {
            assert_eq!(queue, "removed-queue");
            assert_eq!(item_ids, &["dead-orphan-1"]);
        }
        other => panic!("expected QueueRetried for queue without runbook, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::QueueRetry { item_id, .. } if item_id == "dead-orphan-1")));
}

#[test]
fn retry_all_dead_works_without_runbook_definition() {
    let project = project_without_queue();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let ctx = make_ctx(event_bus, Arc::clone(&state));

    for i in 1..=2 {
        let data_map = [("task".to_string(), format!("d{}", i))].into_iter().collect();
        ctx.state.lock().apply_event(&Event::QueuePushed {
            queue: "removed-queue".to_string(),
            item_id: format!("dead-{}", i),
            data: data_map,
            pushed_at_ms: 1_000_000,
            project: String::new(),
        });
        ctx.state.lock().apply_event(&Event::QueueDead {
            queue: "removed-queue".to_string(),
            item_id: format!("dead-{}", i),
            project: String::new(),
        });
    }

    let result = handle_queue_retry(
        &ctx,
        project.path(),
        "",
        "removed-queue",
        RetryFilter { item_ids: &[], all_dead: true, status_filter: None },
    )
    .unwrap();

    match result {
        Response::QueueRetried { ref queue, ref item_ids, .. } => {
            assert_eq!(queue, "removed-queue");
            assert_eq!(item_ids.len(), 2);
        }
        other => panic!("expected QueueRetried for queue without runbook, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 2);
}
