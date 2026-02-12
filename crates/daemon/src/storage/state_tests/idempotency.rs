// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn job_advanced_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&job_transition_event("job-1", "plan"));

    let history_len = state.jobs["job-1"].step_history.len();

    // Apply the same transition again (simulates WAL round-trip double-apply)
    state.apply_event(&job_transition_event("job-1", "plan"));

    // Step history should NOT grow — the duplicate is a no-op
    assert_eq!(state.jobs["job-1"].step_history.len(), history_len);
    assert_eq!(state.jobs["job-1"].step, "plan");
}

#[test]
fn step_completed_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&Event::StepCompleted {
        job_id: JobId::new("job-1"),
        step: "init".to_string(),
    });

    let finished_at = state.jobs["job-1"].step_history[0].finished_at_ms;

    state.apply_event(&Event::StepCompleted {
        job_id: JobId::new("job-1"),
        step: "init".to_string(),
    });

    assert_eq!(state.jobs["job-1"].step_history[0].finished_at_ms, finished_at);
}

#[test]
fn step_failed_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&job_create_event("job-1", "build", "test", "init"));
    state.apply_event(&step_failed_event("job-1", "init", "boom"));

    let finished_at = state.jobs["job-1"].step_history[0].finished_at_ms;

    state.apply_event(&step_failed_event("job-1", "init", "boom"));

    assert_eq!(state.jobs["job-1"].step_history[0].finished_at_ms, finished_at);
}

#[test]
fn worker_item_dispatched_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&worker_start_event("fixer", ""));
    state.apply_event(&Event::WorkerDispatched {
        worker: "fixer".to_string(),
        item_id: "item-1".to_string(),
        owner: JobId::new("job-1").into(),
        project: String::new(),
    });

    assert_eq!(state.workers["fixer"].active.len(), 1);

    // Apply again — should not add a duplicate
    state.apply_event(&Event::WorkerDispatched {
        worker: "fixer".to_string(),
        item_id: "item-1".to_string(),
        owner: JobId::new("job-1").into(),
        project: String::new(),
    });

    assert_eq!(state.workers["fixer"].active.len(), 1);
}

#[test]
fn queue_pushed_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    assert_eq!(state.queue_items["bugs"].len(), 1);

    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    assert_eq!(state.queue_items["bugs"].len(), 1);
}

#[test]
fn queue_failed_idempotent() {
    let mut state = MaterializedState::default();
    state.apply_event(&queue_pushed_event("bugs", "item-1"));
    state.apply_event(&queue_taken_event("bugs", "item-1", "fixer"));

    assert_eq!(state.queue_items["bugs"][0].failures, 0);
    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Active);

    state.apply_event(&queue_failed_event("bugs", "item-1", "job failed"));
    assert_eq!(state.queue_items["bugs"][0].failures, 1);
    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Failed);

    // Second apply — should NOT increment again (idempotent)
    state.apply_event(&queue_failed_event("bugs", "item-1", "job failed"));
    assert_eq!(
        state.queue_items["bugs"][0].failures, 1,
        "failures must not double-increment on idempotent re-apply"
    );
    assert_eq!(state.queue_items["bugs"][0].status, QueueItemStatus::Failed);
}
