// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::Event;
use oj_storage::MaterializedState;

use crate::protocol::Response;

use super::super::handle_queue_retry;
use super::{
    drain_events, project_with_queue_only, push_and_mark_dead, push_and_mark_failed, test_event_bus,
};

// ── Single-item retry tests ───────────────────────────────────────────

#[test]
fn retry_with_prefix_resolves_unique_match() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_dead(
        &state,
        "",
        "tasks",
        "def98765-0000-0000-0000-000000000000",
        &[("task", "retry-me")],
    );

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &["def98".to_string()],
        false,
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueRetried { ref item_id, .. }
            if item_id == "def98765-0000-0000-0000-000000000000"
        ),
        "expected QueueRetried with full ID, got {:?}",
        result
    );

    let events = drain_events(&wal);
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::QueueItemRetry { item_id, .. } if item_id == "def98765-0000-0000-0000-000000000000")));
}

#[test]
fn retry_with_exact_id_still_works() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    push_and_mark_dead(
        &state,
        "",
        "tasks",
        "exact-id-1234",
        &[("task", "retry-me")],
    );

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &["exact-id-1234".to_string()],
        false,
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueRetried { ref item_id, .. }
            if item_id == "exact-id-1234"
        ),
        "expected QueueRetried, got {:?}",
        result
    );
}

#[test]
fn retry_ambiguous_prefix_returns_error() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    for suffix in ["aaa", "bbb"] {
        push_and_mark_dead(
            &state,
            "",
            "tasks",
            &format!("abc-{}", suffix),
            &[("task", "test")],
        );
    }

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &["abc".to_string()],
        false,
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("ambiguous")),
        "expected ambiguous error, got {:?}",
        result
    );
}

#[test]
fn retry_no_match_returns_not_found() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &["nonexistent".to_string()],
        false,
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("not found")),
        "expected not found error, got {:?}",
        result
    );
}

// ── Bulk retry tests ──────────────────────────────────────────────────

#[test]
fn retry_bulk_multiple_items() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // Create multiple dead items
    for i in 1..=3 {
        push_and_mark_dead(
            &state,
            "",
            "tasks",
            &format!("item-{}", i),
            &[("task", "test")],
        );
    }

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &[
            "item-1".to_string(),
            "item-2".to_string(),
            "item-3".to_string(),
        ],
        false,
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    match result {
        Response::QueueItemsRetried {
            ref queue_name,
            ref item_ids,
            ref already_retried,
            ref not_found,
        } => {
            assert_eq!(queue_name, "tasks");
            assert_eq!(item_ids.len(), 3);
            assert!(item_ids.contains(&"item-1".to_string()));
            assert!(item_ids.contains(&"item-2".to_string()));
            assert!(item_ids.contains(&"item-3".to_string()));
            assert!(already_retried.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected QueueItemsRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 3, "expected 3 QueueItemRetry events");
    for event in &events {
        assert!(
            matches!(event, Event::QueueItemRetry { queue_name, .. } if queue_name == "tasks"),
            "expected QueueItemRetry, got {:?}",
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

    // Create multiple items in different states
    push_and_mark_dead(&state, "", "tasks", "dead-1", &[("task", "d1")]);
    push_and_mark_dead(&state, "", "tasks", "dead-2", &[("task", "d2")]);
    // One pending item (should be skipped)
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "p1".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &[],  // Empty - using --all-dead flag
        true, // all_dead = true
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    match result {
        Response::QueueItemsRetried {
            ref queue_name,
            ref item_ids,
            ref already_retried,
            ref not_found,
        } => {
            assert_eq!(queue_name, "tasks");
            assert_eq!(item_ids.len(), 2, "only dead items should be retried");
            assert!(item_ids.contains(&"dead-1".to_string()));
            assert!(item_ids.contains(&"dead-2".to_string()));
            assert!(already_retried.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected QueueItemsRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 2, "expected 2 QueueItemRetry events");
}

#[test]
fn retry_by_status_failed() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // Create items in different states
    push_and_mark_dead(&state, "", "tasks", "dead-1", &[("task", "d1")]);
    push_and_mark_failed(
        &state,
        "",
        "tasks",
        "failed-1",
        &[("task", "f1")],
        1_000_000,
    );
    push_and_mark_failed(
        &state,
        "",
        "tasks",
        "failed-2",
        &[("task", "f2")],
        1_000_000,
    );

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &[], // Empty - using --status flag
        false,
        Some("failed"), // status filter
        &event_bus,
        &state,
    )
    .unwrap();

    match result {
        Response::QueueItemsRetried {
            ref queue_name,
            ref item_ids,
            ref already_retried,
            ref not_found,
        } => {
            assert_eq!(queue_name, "tasks");
            assert_eq!(item_ids.len(), 2, "only failed items should be retried");
            assert!(item_ids.contains(&"failed-1".to_string()));
            assert!(item_ids.contains(&"failed-2".to_string()));
            assert!(
                !item_ids.contains(&"dead-1".to_string()),
                "dead items should not be included"
            );
            assert!(already_retried.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected QueueItemsRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 2, "expected 2 QueueItemRetry events");
}

#[test]
fn retry_bulk_mixed_results() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    // Create one dead item (can be retried)
    push_and_mark_dead(&state, "", "tasks", "dead-1", &[("task", "d1")]);
    // Create one pending item (cannot be retried - not dead/failed)
    state.lock().apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "pending-1".to_string(),
        data: [("task".to_string(), "p1".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    });
    // "nonexistent" doesn't exist

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &[
            "dead-1".to_string(),
            "pending-1".to_string(),
            "nonexistent".to_string(),
        ],
        false,
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    match result {
        Response::QueueItemsRetried {
            ref queue_name,
            ref item_ids,
            ref already_retried,
            ref not_found,
        } => {
            assert_eq!(queue_name, "tasks");
            assert_eq!(item_ids, &["dead-1".to_string()]);
            assert_eq!(already_retried, &["pending-1".to_string()]);
            assert_eq!(not_found, &["nonexistent".to_string()]);
        }
        other => panic!("expected QueueItemsRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert_eq!(events.len(), 1, "only 1 QueueItemRetry event for dead-1");
}

#[test]
fn retry_all_dead_empty_queue() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, wal, _) = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_queue_retry(
        project.path(),
        "",
        "tasks",
        &[],
        true, // all_dead = true
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    match result {
        Response::QueueItemsRetried {
            ref queue_name,
            ref item_ids,
            ref already_retried,
            ref not_found,
        } => {
            assert_eq!(queue_name, "tasks");
            assert!(item_ids.is_empty());
            assert!(already_retried.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected empty QueueItemsRetried, got {:?}", other),
    }

    let events = drain_events(&wal);
    assert!(events.is_empty());
}

// ── Retry namespace fallback test ─────────────────────────────────────

#[test]
fn retry_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_queue_only();
    let wal_dir = tempdir().unwrap();
    let (event_bus, _wal, _) = test_event_bus(wal_dir.path());

    // Pre-populate state with a worker that knows the real project root
    let mut initial = MaterializedState::default();
    initial.workers.insert(
        "my-project/processor".to_string(),
        oj_storage::WorkerRecord {
            name: "processor".to_string(),
            project_root: project.path().to_path_buf(),
            runbook_hash: "fake-hash".to_string(),
            status: "stopped".to_string(),
            active_job_ids: vec![],
            queue_name: "tasks".to_string(),
            concurrency: 1,
            namespace: "my-project".to_string(),
        },
    );
    // Add a dead queue item to retry
    push_and_mark_dead(
        &Arc::new(Mutex::new(MaterializedState::default())),
        "my-project",
        "tasks",
        "item-dead-1",
        &[("task", "retry-me")],
    );
    // Apply directly to initial state
    initial.apply_event(&Event::QueuePushed {
        queue_name: "tasks".to_string(),
        item_id: "item-dead-1".to_string(),
        data: [("task".to_string(), "retry-me".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: "my-project".to_string(),
    });
    initial.apply_event(&Event::QueueItemDead {
        queue_name: "tasks".to_string(),
        item_id: "item-dead-1".to_string(),
        namespace: "my-project".to_string(),
    });
    let state = Arc::new(Mutex::new(initial));

    let result = handle_queue_retry(
        std::path::Path::new("/wrong/path"),
        "my-project",
        "tasks",
        &["item-dead-1".to_string()],
        false,
        None,
        &event_bus,
        &state,
    )
    .unwrap();

    assert!(
        matches!(
            result,
            Response::QueueRetried { ref queue_name, ref item_id }
            if queue_name == "tasks" && item_id == "item-dead-1"
        ),
        "expected QueueRetried from namespace fallback, got {:?}",
        result
    );
}
