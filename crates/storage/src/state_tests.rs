// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{Event, PipelineId, SessionId, StepOutcome, WorkspaceId};

fn pipeline_create_event(id: &str, kind: &str, name: &str, initial_step: &str) -> Event {
    Event::PipelineCreated {
        id: PipelineId::new(id),
        kind: kind.to_string(),
        name: name.to_string(),
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/test/project"),
        vars: HashMap::new(),
        initial_step: initial_step.to_string(),
        created_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    }
}

fn pipeline_delete_event(id: &str) -> Event {
    Event::PipelineDeleted {
        id: PipelineId::new(id),
    }
}

fn pipeline_transition_event(id: &str, step: &str) -> Event {
    Event::PipelineAdvanced {
        id: PipelineId::new(id),
        step: step.to_string(),
    }
}

fn step_started_event(pipeline_id: &str) -> Event {
    Event::StepStarted {
        pipeline_id: PipelineId::new(pipeline_id),
        step: "init".to_string(),
        agent_id: None,
        agent_name: None,
    }
}

fn session_create_event(id: &str, pipeline_id: &str) -> Event {
    Event::SessionCreated {
        id: SessionId::new(id),
        pipeline_id: PipelineId::new(pipeline_id),
    }
}

fn session_delete_event(id: &str) -> Event {
    Event::SessionDeleted {
        id: SessionId::new(id),
    }
}

fn workspace_create_event(
    id: &str,
    path: &str,
    branch: Option<&str>,
    owner: Option<&str>,
) -> Event {
    Event::WorkspaceCreated {
        id: WorkspaceId::new(id),
        path: PathBuf::from(path),
        branch: branch.map(String::from),
        owner: owner.map(String::from),
        mode: None,
    }
}

fn workspace_ready_event(id: &str) -> Event {
    Event::WorkspaceReady {
        id: WorkspaceId::new(id),
    }
}

fn workspace_delete_event(id: &str) -> Event {
    Event::WorkspaceDeleted {
        id: WorkspaceId::new(id),
    }
}

#[test]
fn apply_event_pipeline_create() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));

    assert!(state.pipelines.contains_key("pipe-1"));
}

#[test]
fn apply_event_pipeline_delete() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&pipeline_delete_event("pipe-1"));

    assert!(!state.pipelines.contains_key("pipe-1"));
}

#[test]
fn apply_event_workspace_lifecycle() {
    let mut state = MaterializedState::default();
    state.apply_event(&workspace_create_event(
        "ws-1",
        "/tmp/test",
        Some("feature/test"),
        Some("pipe-1"),
    ));

    assert!(state.workspaces.contains_key("ws-1"));
    assert_eq!(state.workspaces["ws-1"].path, PathBuf::from("/tmp/test"));
    assert_eq!(
        state.workspaces["ws-1"].branch,
        Some("feature/test".to_string())
    );
    assert_eq!(state.workspaces["ws-1"].owner, Some("pipe-1".to_string()));
    assert_eq!(
        state.workspaces["ws-1"].status,
        oj_core::WorkspaceStatus::Creating
    );

    // Update status to Ready
    state.apply_event(&workspace_ready_event("ws-1"));
    assert_eq!(
        state.workspaces["ws-1"].status,
        oj_core::WorkspaceStatus::Ready
    );

    state.apply_event(&workspace_delete_event("ws-1"));
    assert!(!state.workspaces.contains_key("ws-1"));
}

#[test]
fn get_pipeline_exact_match() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event(
        "pipe-abc123",
        "build",
        "test",
        "init",
    ));

    assert!(state.get_pipeline("pipe-abc123").is_some());
    assert_eq!(state.get_pipeline("pipe-abc123").unwrap().id, "pipe-abc123");
}

#[test]
fn get_pipeline_prefix_match() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event(
        "pipe-abc123",
        "build",
        "test",
        "init",
    ));

    assert!(state.get_pipeline("pipe-abc").is_some());
    assert_eq!(state.get_pipeline("pipe-abc").unwrap().id, "pipe-abc123");
}

#[test]
fn get_pipeline_ambiguous_prefix() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event(
        "pipe-abc123",
        "build",
        "test1",
        "init",
    ));
    state.apply_event(&pipeline_create_event(
        "pipe-abc456",
        "build",
        "test2",
        "init",
    ));

    // "pipe-abc" matches both, so returns None
    assert!(state.get_pipeline("pipe-abc").is_none());
}

#[test]
fn get_pipeline_no_match() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event(
        "pipe-abc123",
        "build",
        "test",
        "init",
    ));

    assert!(state.get_pipeline("pipe-xyz").is_none());
}

#[test]
fn apply_event_pipeline_transition() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));

    assert_eq!(state.pipelines["pipe-1"].step, "init");

    state.apply_event(&pipeline_transition_event("pipe-1", "build"));

    assert_eq!(state.pipelines["pipe-1"].step, "build");
    assert_eq!(
        state.pipelines["pipe-1"].step_status,
        oj_core::StepStatus::Pending
    );
}

#[test]
fn apply_event_step_started() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));

    state.apply_event(&step_started_event("pipe-1"));

    assert_eq!(
        state.pipelines["pipe-1"].step_status,
        oj_core::StepStatus::Running
    );
}

#[test]
fn apply_event_step_waiting_with_reason_sets_pipeline_error() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));

    assert!(state.pipelines["pipe-1"].error.is_none());

    state.apply_event(&Event::StepWaiting {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
        reason: Some("gate `make test` failed (exit 1): tests failed".to_string()),
    });

    assert_eq!(
        state.pipelines["pipe-1"].step_status,
        oj_core::StepStatus::Waiting
    );
    assert_eq!(
        state.pipelines["pipe-1"].error.as_deref(),
        Some("gate `make test` failed (exit 1): tests failed")
    );
}

#[test]
fn apply_event_step_started_preserves_existing_error() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));

    // Set an error via StepWaiting
    state.apply_event(&Event::StepWaiting {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
        reason: Some("previous error".to_string()),
    });

    // StepStarted should not clear existing error
    state.apply_event(&step_started_event("pipe-1"));

    assert_eq!(
        state.pipelines["pipe-1"].error.as_deref(),
        Some("previous error")
    );
}

#[test]
fn apply_event_session_lifecycle() {
    let mut state = MaterializedState::default();
    state.apply_event(&session_create_event("sess-1", "pipe-1"));

    assert!(state.sessions.contains_key("sess-1"));
    assert_eq!(state.sessions["sess-1"].pipeline_id, "pipe-1");

    state.apply_event(&session_delete_event("sess-1"));

    assert!(!state.sessions.contains_key("sess-1"));
}

// === Step history tests ===

#[test]
fn step_history_initialized_on_create() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));

    let pipeline = &state.pipelines["pipe-1"];
    assert_eq!(pipeline.step_history.len(), 1);
    assert_eq!(pipeline.step_history[0].name, "init");
    assert!(pipeline.step_history[0].finished_at_ms.is_none());
    assert_eq!(pipeline.step_history[0].outcome, StepOutcome::Running);
}

#[test]
fn step_history_transition_appends_record() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&pipeline_transition_event("pipe-1", "plan"));

    let pipeline = &state.pipelines["pipe-1"];
    assert_eq!(pipeline.step_history.len(), 2);

    // First step finalized as completed
    assert_eq!(pipeline.step_history[0].name, "init");
    assert!(pipeline.step_history[0].finished_at_ms.is_some());
    assert_eq!(pipeline.step_history[0].outcome, StepOutcome::Completed);

    // New step started
    assert_eq!(pipeline.step_history[1].name, "plan");
    assert!(pipeline.step_history[1].finished_at_ms.is_none());
    assert_eq!(pipeline.step_history[1].outcome, StepOutcome::Running);
}

#[test]
fn step_history_waiting_sets_outcome() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&Event::StepWaiting {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
        reason: Some("gate failed: exit 2".to_string()),
    });

    let pipeline = &state.pipelines["pipe-1"];
    assert_eq!(pipeline.step_history.len(), 1);
    assert!(pipeline.step_history[0].finished_at_ms.is_none()); // still open
    assert_eq!(
        pipeline.step_history[0].outcome,
        StepOutcome::Waiting("gate failed: exit 2".to_string())
    );
}

#[test]
fn step_history_multi_step_sequence() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&pipeline_transition_event("pipe-1", "plan"));
    state.apply_event(&pipeline_transition_event("pipe-1", "implement"));
    state.apply_event(&pipeline_transition_event("pipe-1", "done"));

    let pipeline = &state.pipelines["pipe-1"];
    assert_eq!(pipeline.step_history.len(), 3); // init, plan, implement (done is terminal)

    assert_eq!(pipeline.step_history[0].name, "init");
    assert_eq!(pipeline.step_history[0].outcome, StepOutcome::Completed);
    assert!(pipeline.step_history[0].finished_at_ms.is_some());

    assert_eq!(pipeline.step_history[1].name, "plan");
    assert_eq!(pipeline.step_history[1].outcome, StepOutcome::Completed);
    assert!(pipeline.step_history[1].finished_at_ms.is_some());

    assert_eq!(pipeline.step_history[2].name, "implement");
    assert_eq!(pipeline.step_history[2].outcome, StepOutcome::Completed);
    assert!(pipeline.step_history[2].finished_at_ms.is_some());
}

#[test]
fn step_history_shell_completed_success() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&Event::ShellExited {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
        exit_code: 0,
    });

    let pipeline = &state.pipelines["pipe-1"];
    assert!(pipeline.step_history[0].finished_at_ms.is_some());
    assert_eq!(pipeline.step_history[0].outcome, StepOutcome::Completed);
}

#[test]
fn step_history_shell_completed_failure() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&Event::ShellExited {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
        exit_code: 42,
    });

    let pipeline = &state.pipelines["pipe-1"];
    assert!(pipeline.step_history[0].finished_at_ms.is_some());
    assert_eq!(
        pipeline.step_history[0].outcome,
        StepOutcome::Failed("shell exit code: 42".to_string())
    );
}

#[test]
fn step_history_serde_roundtrip() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&pipeline_transition_event("pipe-1", "plan"));

    // Serialize and deserialize
    let json = serde_json::to_string(&state).unwrap();
    let restored: MaterializedState = serde_json::from_str(&json).unwrap();

    let pipeline = &restored.pipelines["pipe-1"];
    assert_eq!(pipeline.step_history.len(), 2);
    assert_eq!(pipeline.step_history[0].name, "init");
    assert_eq!(pipeline.step_history[0].outcome, StepOutcome::Completed);
    assert_eq!(pipeline.step_history[1].name, "plan");
    assert_eq!(pipeline.step_history[1].outcome, StepOutcome::Running);
}

#[test]
fn step_history_backward_compat_empty_on_old_snapshot() {
    // Simulate an old snapshot without step_history by deserializing JSON without it
    let json = r#"{
        "pipelines": {
            "pipe-old": {
                "id": "pipe-old",
                "name": "legacy",
                "kind": "build",
                "step": "init",
                "step_status": "Running",
                "input": {},
                "runbook_hash": "abc",
                "cwd": "/tmp",
                "workspace_id": null,
                "workspace_path": null,
                "session_id": null,
                "error": null
            }
        },
        "sessions": {},
        "workspaces": {},
        "workers": {},
        "runbooks": {}
    }"#;

    let state: MaterializedState = serde_json::from_str(json).unwrap();
    let pipeline = &state.pipelines["pipe-old"];
    assert!(pipeline.step_history.is_empty());
}

#[test]
fn apply_event_worker_started_with_queue_and_concurrency() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::WorkerStarted {
        worker_name: "fixer".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        queue_name: "bugs".to_string(),
        concurrency: 3,
        namespace: String::new(),
    });
    let worker = &state.workers["fixer"];
    assert_eq!(worker.status, "running");
    assert_eq!(worker.queue_name, "bugs");
    assert_eq!(worker.concurrency, 3);
    assert!(worker.active_pipeline_ids.is_empty());
}

#[test]
fn apply_event_worker_stopped_sets_status() {
    let mut state = MaterializedState::default();
    state.apply_event(&worker_start_event("fixer", ""));
    assert_eq!(state.workers["fixer"].status, "running");

    state.apply_event(&Event::WorkerStopped {
        worker_name: "fixer".to_string(),
        namespace: String::new(),
    });
    assert_eq!(state.workers["fixer"].status, "stopped");
}

#[test]
fn worker_record_backward_compat_missing_fields() {
    // Simulate an old snapshot without queue_name and concurrency
    let json = r#"{
        "pipelines": {},
        "sessions": {},
        "workspaces": {},
        "workers": {
            "old-worker": {
                "name": "old-worker",
                "project_root": "/tmp",
                "runbook_hash": "abc",
                "status": "running",
                "active_pipeline_ids": []
            }
        },
        "runbooks": {}
    }"#;

    let state: MaterializedState = serde_json::from_str(json).unwrap();
    let worker = &state.workers["old-worker"];
    assert_eq!(worker.queue_name, "");
    assert_eq!(worker.concurrency, 0);
}

#[test]
fn cancelled_pipeline_is_terminal_after_event_replay() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "execute"));
    state.apply_event(&step_started_event("pipe-1"));

    // Apply cancellation events (as they would appear in WAL replay after daemon restart)
    state.apply_event(&pipeline_transition_event("pipe-1", "cancelled"));
    state.apply_event(&Event::StepFailed {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "execute".to_string(),
        error: "cancelled".to_string(),
    });

    let pipeline = &state.pipelines["pipe-1"];
    assert!(pipeline.is_terminal());
    assert_eq!(pipeline.step, "cancelled");
    assert_eq!(pipeline.step_status, oj_core::StepStatus::Failed);
    assert_eq!(pipeline.error.as_deref(), Some("cancelled"));
}

#[test]
fn worker_started_preserves_active_pipeline_ids_on_restart() {
    let mut state = MaterializedState::default();

    // Simulate pre-restart state: worker with active pipelines
    state.apply_event(&worker_start_event("fixer", ""));
    state.apply_event(&Event::WorkerItemDispatched {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        pipeline_id: PipelineId::new("pipe-1"),
        namespace: String::new(),
    });
    state.apply_event(&Event::WorkerItemDispatched {
        worker_name: "fixer".to_string(),
        item_id: "item-2".to_string(),
        pipeline_id: PipelineId::new("pipe-2"),
        namespace: String::new(),
    });

    assert_eq!(state.workers["fixer"].active_pipeline_ids.len(), 2);

    // Simulate daemon restart: WorkerStarted replayed from WAL
    state.apply_event(&worker_start_event("fixer", ""));

    // Active pipeline IDs must be preserved
    let worker = &state.workers["fixer"];
    assert_eq!(worker.active_pipeline_ids.len(), 2);
    assert!(worker.active_pipeline_ids.contains(&"pipe-1".to_string()));
    assert!(worker.active_pipeline_ids.contains(&"pipe-2".to_string()));
}

// === Idempotency tests ===

#[test]
fn apply_event_pipeline_advanced_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&pipeline_transition_event("pipe-1", "plan"));

    let history_len = state.pipelines["pipe-1"].step_history.len();

    // Apply the same transition again (simulates WAL round-trip double-apply)
    state.apply_event(&pipeline_transition_event("pipe-1", "plan"));

    // Step history should NOT grow — the duplicate is a no-op
    assert_eq!(state.pipelines["pipe-1"].step_history.len(), history_len);
    assert_eq!(state.pipelines["pipe-1"].step, "plan");
}

#[test]
fn apply_event_step_completed_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&Event::StepCompleted {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
    });

    let finished_at = state.pipelines["pipe-1"].step_history[0].finished_at_ms;

    // Apply again — finalize_current_step is already guarded by finished_at_ms
    state.apply_event(&Event::StepCompleted {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
    });

    // finished_at should be unchanged (not overwritten)
    assert_eq!(
        state.pipelines["pipe-1"].step_history[0].finished_at_ms,
        finished_at
    );
}

#[test]
fn apply_event_step_failed_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&pipeline_create_event("pipe-1", "build", "test", "init"));
    state.apply_event(&Event::StepFailed {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
        error: "boom".to_string(),
    });

    let finished_at = state.pipelines["pipe-1"].step_history[0].finished_at_ms;

    // Apply again — finalize_current_step is already guarded by finished_at_ms
    state.apply_event(&Event::StepFailed {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "init".to_string(),
        error: "boom".to_string(),
    });

    assert_eq!(
        state.pipelines["pipe-1"].step_history[0].finished_at_ms,
        finished_at
    );
}

#[test]
fn apply_event_worker_item_dispatched_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&worker_start_event("fixer", ""));
    state.apply_event(&Event::WorkerItemDispatched {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        pipeline_id: PipelineId::new("pipe-1"),
        namespace: String::new(),
    });

    assert_eq!(state.workers["fixer"].active_pipeline_ids.len(), 1);

    // Apply again — should not add a duplicate
    state.apply_event(&Event::WorkerItemDispatched {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        pipeline_id: PipelineId::new("pipe-1"),
        namespace: String::new(),
    });

    assert_eq!(state.workers["fixer"].active_pipeline_ids.len(), 1);
}

#[test]
fn apply_event_queue_pushed_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    assert_eq!(state.queue_items["bugs"].len(), 1);

    // Apply again — should not add a duplicate
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    assert_eq!(state.queue_items["bugs"].len(), 1);
}

// === Queue event tests ===

fn queue_pushed_event(queue_name: &str, item_id: &str) -> Event {
    Event::QueuePushed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        data: [
            ("title".to_string(), "Fix bug".to_string()),
            ("id".to_string(), "123".to_string()),
        ]
        .into_iter()
        .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    }
}

fn queue_taken_event(queue_name: &str, item_id: &str, worker_name: &str) -> Event {
    Event::QueueTaken {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        worker_name: worker_name.to_string(),
        namespace: String::new(),
    }
}

fn queue_completed_event(queue_name: &str, item_id: &str) -> Event {
    Event::QueueCompleted {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: String::new(),
    }
}

fn queue_failed_event(queue_name: &str, item_id: &str, error: &str) -> Event {
    Event::QueueFailed {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        error: error.to_string(),
        namespace: String::new(),
    }
}

#[test]
fn queue_pushed_creates_pending_item() {
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
fn queue_taken_marks_active() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));

    let items = &state.queue_items["bugs"];
    assert_eq!(items[0].status, QueueItemStatus::Active);
    assert_eq!(items[0].worker_name.as_deref(), Some("fixer"));
}

#[test]
fn queue_completed_marks_completed() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_completed_event("bugs", "item-1"));

    let items = &state.queue_items["bugs"];
    assert_eq!(items[0].status, QueueItemStatus::Completed);
}

#[test]
fn queue_failed_marks_failed() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "pipeline failed"));

    let items = &state.queue_items["bugs"];
    assert_eq!(items[0].status, QueueItemStatus::Failed);
}

fn queue_dropped_event(queue_name: &str, item_id: &str) -> Event {
    Event::QueueDropped {
        queue_name: queue_name.to_string(),
        item_id: item_id.to_string(),
        namespace: String::new(),
    }
}

#[test]
fn queue_dropped_removes_item() {
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
fn queue_dropped_nonexistent_item_is_noop() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    assert_eq!(state.queue_items["bugs"].len(), 1);

    // Drop a non-existent item — should be a no-op
    state.apply_event(&queue_dropped_event("bugs", "item-999"));
    assert_eq!(state.queue_items["bugs"].len(), 1);
}

#[test]
fn queue_dropped_nonexistent_queue_is_noop() {
    let mut state = MaterializedState::default();
    // Drop from a queue that doesn't exist — should be a no-op
    state.apply_event(&queue_dropped_event("nonexistent", "item-1"));
    assert!(!state.queue_items.contains_key("nonexistent"));
}

#[test]
fn queue_pushed_to_nonexistent_queue_creates_it() {
    let mut state = MaterializedState::default();
    assert!(!state.queue_items.contains_key("new-queue"));

    state.apply_event(&queue_pushed_event("new-queue", "item-1"));

    assert!(state.queue_items.contains_key("new-queue"));
    assert_eq!(state.queue_items["new-queue"].len(), 1);
}

// === Dead letter / retry tests ===

#[test]
fn queue_failed_increments_failure_count() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));

    assert_eq!(state.queue_items["bugs"][0].failure_count, 0);

    state.apply_event(&queue_failed_event("bugs", "item-1", "pipeline failed"));
    assert_eq!(state.queue_items["bugs"][0].failure_count, 1);
    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Failed);

    // Simulate retry (back to active, then fail again)
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event(
        "bugs",
        "item-1",
        "pipeline failed again",
    ));
    assert_eq!(state.queue_items["bugs"][0].failure_count, 2);
}

#[test]
fn queue_item_retry_resets_to_pending() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "pipeline failed"));

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
fn queue_item_dead_sets_dead_status() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));
    state.apply_event(&queue_failed_event("bugs", "item-1", "pipeline failed"));

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
fn queue_item_retry_on_dead_item_resets_to_pending() {
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

    // Retry should reset Dead -> Pending
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
    // Simulate an old snapshot without failure_count field
    let json = r#"{
        "pipelines": {},
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

// === Worker delete tests ===

fn worker_start_event(name: &str, ns: &str) -> Event {
    Event::WorkerStarted {
        worker_name: name.to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        queue_name: "queue".to_string(),
        concurrency: 1,
        namespace: ns.to_string(),
    }
}

#[test]
fn apply_event_worker_deleted_lifecycle_and_ghost() {
    let mut state = MaterializedState::default();

    // Namespaced worker: start → stop → delete
    state.apply_event(&worker_start_event("fixer", "myproject"));
    assert_eq!(state.workers["myproject/fixer"].status, "running");
    state.apply_event(&Event::WorkerStopped {
        worker_name: "fixer".to_string(),
        namespace: "myproject".to_string(),
    });
    assert_eq!(state.workers["myproject/fixer"].status, "stopped");
    state.apply_event(&Event::WorkerDeleted {
        worker_name: "fixer".to_string(),
        namespace: "myproject".to_string(),
    });
    assert!(!state.workers.contains_key("myproject/fixer"));

    // Ghost worker (empty namespace): start → delete
    state.apply_event(&worker_start_event("ghost", ""));
    assert!(state.workers.contains_key("ghost"));
    state.apply_event(&Event::WorkerDeleted {
        worker_name: "ghost".to_string(),
        namespace: String::new(),
    });
    assert!(!state.workers.contains_key("ghost"));

    // Delete nonexistent worker is a no-op
    state.apply_event(&Event::WorkerDeleted {
        worker_name: "nonexistent".to_string(),
        namespace: String::new(),
    });
    assert!(state.workers.is_empty());
}
