// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

fn queue_completed_event(queue_name: &str, item_id: &str) -> Event {
    Event::QueueCompleted {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: String::new(),
    }
}

fn queue_dropped_event(queue_name: &str, item_id: &str) -> Event {
    Event::QueueDropped {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: String::new(),
    }
}

// ── Basic queue transitions ──────────────────────────────────────────────────

#[test]
fn pushed_creates_pending_item() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));

    assert!(state.queue_items.contains_key("bugs"));
    let items = &state.queue_items["bugs"];
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "item-1");
    assert_eq!(items[0].queue_name, "bugs");
    assert_eq!(items[0].status, QueueItemStatus::Pending);
    assert!(items[0].worker_name.is_none());
    assert_eq!(items[0].data["title"], "Fix bug");
    assert_eq!(items[0].pushed_at_epoch_ms, 1_000_000);
}

#[test]
fn taken_marks_active() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));

    let items = &state.queue_items["bugs"];
    assert_eq!(items[0].status, QueueItemStatus::Active);
    assert_eq!(items[0].worker_name.as_deref(), Some("fixer"));
}

#[test]
fn completed_marks_completed() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_completed_event("bugs", "item-1"));

    assert_eq!(
        state.queue_items["bugs"][0].status,
        QueueItemStatus::Completed
    );
}

#[test]
fn failed_marks_failed() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "job failed"));

    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Failed);
}

#[test]
fn pushed_to_nonexistent_queue_creates_it() {
    let mut state = MaterializedState::default();
    assert!(!state.queue_items.contains_key("new-queue"));

    state.apply_event(&queue_pushed_event("new-queue", "item-1"));

    assert!(state.queue_items.contains_key("new-queue"));
    assert_eq!(state.queue_items["new-queue"].len(), 1);
}

// ── Drop ─────────────────────────────────────────────────────────────────────

#[test]
fn dropped_removes_item() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_pushed_event("bugs", "item-2"));
    assert_eq!(state.queue_items["bugs"].len(), 2);

    state.apply_event(&queue_dropped_event("bugs", "item-1"));

    let items = &state.queue_items["bugs"];
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "item-2");
}

#[test]
fn dropped_nonexistent_item_is_noop() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    assert_eq!(state.queue_items["bugs"].len(), 1);

    state.apply_event(&queue_dropped_event("bugs", "item-999"));
    assert_eq!(state.queue_items["bugs"].len(), 1);
}

#[test]
fn dropped_nonexistent_queue_is_noop() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_dropped_event("nonexistent", "item-1"));
    assert!(!state.queue_items.contains_key("nonexistent"));
}

// ── Dead letter / retry ──────────────────────────────────────────────────────

#[test]
fn failed_increments_failure_count() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));

    assert_eq!(state.queue_items["bugs"][0].failure_count, 0);

    state.apply_event(&queue_failed_event("bugs", "item-1", "job failed"));
    assert_eq!(state.queue_items["bugs"][0].failure_count, 1);
    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Failed);

    // Simulate retry (back to active, then fail again)
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "job failed again"));
    assert_eq!(state.queue_items["bugs"][0].failure_count, 2);
}

#[test]
fn item_retry_resets_to_pending() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "job failed"));

    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Failed);
    assert_eq!(state.queue_items["bugs"][0].failure_count, 1);
    assert_eq!(
        state.queue_items["bugs"][0].worker_name.as_deref(),
        Some("fixer")
    );

    state.apply_event(&Event::QueueItemRetry {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        namespace: String::new(),
    });

    assert_eq!(
        state.queue_items["bugs"][0].status,
        QueueItemStatus::Pending
    );
    assert_eq!(state.queue_items["bugs"][0].failure_count, 0);
    assert!(state.queue_items["bugs"][0].worker_name.is_none());
}

#[test]
fn item_dead_sets_dead_status() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "job failed"));

    state.apply_event(&Event::QueueItemDead {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        namespace: String::new(),
    });

    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Dead);
}

#[test]
fn dead_status_serde_roundtrip() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "err"));
    state.apply_event(&Event::QueueItemDead {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        namespace: String::new(),
    });

    let json = serde_json::to_string(&state).expect("serialize");
    let restored: MaterializedState = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        restored.queue_items["bugs"][0].status,
        QueueItemStatus::Dead
    );
    assert_eq!(restored.queue_items["bugs"][0].failure_count, 1);
}

#[test]
fn item_retry_on_dead_resets_to_pending() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "err"));
    state.apply_event(&Event::QueueItemDead {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        namespace: String::new(),
    });

    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Dead);

    state.apply_event(&Event::QueueItemRetry {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        namespace: String::new(),
    });

    assert_eq!(
        state.queue_items["bugs"][0].status,
        QueueItemStatus::Pending
    );
    assert_eq!(state.queue_items["bugs"][0].failure_count, 0);
    assert!(state.queue_items["bugs"][0].worker_name.is_none());
}

#[test]
fn failure_count_backward_compat_defaults_to_zero() {
    let json = r#"{
        "jobs": {},
        "sessions": {},
        "workspaces": {},
        "workers": {},
        "runbooks": {},
        "queue_items": {
            "bugs": [{
                "id": "item-old",
                "queue_name": "bugs",
                "data": {"title": "old bug"},
                "status": "failed",
                "worker_name": null,
                "pushed_at_epoch_ms": 1000000
            }]
        }
    }"#;

    let state: MaterializedState = serde_json::from_str(json).expect("deserialize");
    assert_eq!(state.queue_items["bugs"][0].failure_count, 0);
}
