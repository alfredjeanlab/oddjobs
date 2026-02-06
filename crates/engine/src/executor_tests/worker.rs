// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for worker effects (PollQueue, TakeQueueItem).

use super::*;

// === PollQueue tests ===

#[tokio::test]
async fn poll_queue_with_valid_json() {
    let mut harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::PollQueue {
            worker_name: "poller".to_string(),
            list_command: r#"echo '[{"id":"1"},{"id":"2"}]'"#.to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        })
        .await
        .unwrap();

    assert!(result.is_none(), "PollQueue returns None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerPollComplete {
            worker_name, items, ..
        } => {
            assert_eq!(worker_name, "poller");
            assert_eq!(items.len(), 2);
            assert_eq!(items[0]["id"], "1");
            assert_eq!(items[1]["id"], "2");
        }
        other => panic!("expected WorkerPollComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn poll_queue_with_empty_output() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::PollQueue {
            worker_name: "poller".to_string(),
            list_command: "echo '[]'".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerPollComplete { items, .. } => {
            assert!(items.is_empty());
        }
        other => panic!("expected WorkerPollComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn poll_queue_with_invalid_json_returns_empty() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::PollQueue {
            worker_name: "poller".to_string(),
            list_command: "echo 'not json'".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerPollComplete { items, .. } => {
            assert!(
                items.is_empty(),
                "invalid JSON should result in empty items"
            );
        }
        other => panic!("expected WorkerPollComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn poll_queue_command_failure_returns_empty() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::PollQueue {
            worker_name: "poller".to_string(),
            list_command: "exit 1".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerPollComplete { items, .. } => {
            assert!(
                items.is_empty(),
                "failed command should result in empty items"
            );
        }
        other => panic!("expected WorkerPollComplete, got {:?}", other),
    }
}

// === TakeQueueItem tests ===

#[tokio::test]
async fn take_queue_item_effect_runs_async() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::TakeQueueItem {
            worker_name: "test-worker".to_string(),
            take_command: "echo taken".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            item_id: "item-1".to_string(),
            item: serde_json::json!({"id": "item-1", "title": "test"}),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "TakeQueueItem should return None (async)");

    // WorkerTakeComplete arrives via event_tx
    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerTakeComplete {
            worker_name,
            item_id,
            exit_code,
            ..
        } => {
            assert_eq!(worker_name, "test-worker");
            assert_eq!(item_id, "item-1");
            assert_eq!(exit_code, 0);
        }
        other => panic!("expected WorkerTakeComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn take_queue_item_failure_returns_nonzero() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::TakeQueueItem {
            worker_name: "test-worker".to_string(),
            take_command: "exit 1".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            item_id: "item-2".to_string(),
            item: serde_json::json!({"id": "item-2"}),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "TakeQueueItem should return None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerTakeComplete {
            exit_code, item_id, ..
        } => {
            assert_eq!(item_id, "item-2");
            assert_ne!(exit_code, 0, "failed take should have nonzero exit code");
        }
        other => panic!("expected WorkerTakeComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn take_queue_item_with_stderr() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::TakeQueueItem {
            worker_name: "test-worker".to_string(),
            take_command: "echo stderr_msg >&2 && exit 1".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            item_id: "item-3".to_string(),
            item: serde_json::json!({"id": "item-3"}),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerTakeComplete {
            exit_code, stderr, ..
        } => {
            assert_ne!(exit_code, 0);
            assert!(stderr.is_some());
            assert!(stderr.unwrap().contains("stderr_msg"));
        }
        other => panic!("expected WorkerTakeComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn take_queue_item_preserves_item_data() {
    let mut harness = setup().await;

    let item_data = serde_json::json!({
        "id": "item-4",
        "title": "Important task",
        "priority": 1
    });

    harness
        .executor
        .execute(Effect::TakeQueueItem {
            worker_name: "test-worker".to_string(),
            take_command: "true".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            item_id: "item-4".to_string(),
            item: item_data.clone(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::WorkerTakeComplete { item, item_id, .. } => {
            assert_eq!(item_id, "item-4");
            assert_eq!(item, item_data);
        }
        other => panic!("expected WorkerTakeComplete, got {:?}", other),
    }
}
