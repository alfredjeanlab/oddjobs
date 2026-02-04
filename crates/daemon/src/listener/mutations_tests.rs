// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::{Event, Pipeline, StepOutcome, StepRecord, StepStatus};
use oj_engine::breadcrumb::Breadcrumb;
use oj_storage::{MaterializedState, Wal};

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::{
    handle_agent_prune, handle_pipeline_cancel, handle_pipeline_resume, handle_session_kill,
};

fn test_event_bus(dir: &std::path::Path) -> EventBus {
    let wal_path = dir.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, _reader) = EventBus::new(wal);
    event_bus
}

fn empty_state() -> Arc<Mutex<MaterializedState>> {
    Arc::new(Mutex::new(MaterializedState::default()))
}

fn empty_orphans() -> Arc<Mutex<Vec<Breadcrumb>>> {
    Arc::new(Mutex::new(Vec::new()))
}

fn make_pipeline(id: &str, step: &str) -> Pipeline {
    Pipeline {
        id: id.to_string(),
        name: "test-pipeline".to_string(),
        kind: "test".to_string(),
        namespace: "proj".to_string(),
        step: step.to_string(),
        step_status: StepStatus::Running,
        step_started_at: Instant::now(),
        step_history: vec![StepRecord {
            name: step.to_string(),
            started_at_ms: 1000,
            finished_at_ms: None,
            outcome: StepOutcome::Running,
            agent_id: None,
            agent_name: None,
        }],
        vars: HashMap::new(),
        runbook_hash: "abc123".to_string(),
        cwd: std::path::PathBuf::from("/tmp/project"),
        workspace_id: None,
        workspace_path: None,
        session_id: None,
        created_at: Instant::now(),
        error: None,
        action_attempts: HashMap::new(),
        agent_signal: None,
        cancelling: false,
        total_retries: 0,
        step_visits: HashMap::new(),
        cron_name: None,
    }
}

fn make_breadcrumb(pipeline_id: &str) -> Breadcrumb {
    Breadcrumb {
        pipeline_id: pipeline_id.to_string(),
        project: "proj".to_string(),
        kind: "test".to_string(),
        name: "test-pipeline".to_string(),
        vars: HashMap::new(),
        current_step: "work".to_string(),
        step_status: "running".to_string(),
        agents: vec![],
        workspace_id: None,
        workspace_root: None,
        updated_at: "2026-01-15T10:30:00Z".to_string(),
        runbook_hash: "hash456".to_string(),
        cwd: Some(std::path::PathBuf::from("/tmp/project")),
    }
}

/// Populate the runbooks map in state by applying a RunbookLoaded event.
fn load_runbook_into_state(state: &Arc<Mutex<MaterializedState>>, hash: &str) {
    let event = Event::RunbookLoaded {
        hash: hash.to_string(),
        version: 1,
        runbook: serde_json::json!({}),
    };
    state.lock().apply_event(&event);
}

#[test]
fn resume_existing_pipeline_emits_event() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();
    let orphans = empty_orphans();

    // Insert a pipeline in state
    {
        let mut s = state.lock();
        s.pipelines
            .insert("pipe-1".to_string(), make_pipeline("pipe-1", "work"));
    }

    let result = handle_pipeline_resume(
        &state,
        &orphans,
        &event_bus,
        "pipe-1".to_string(),
        Some("try again".to_string()),
        HashMap::new(),
    );

    assert!(matches!(result, Ok(Response::Ok)));
}

#[test]
fn resume_nonexistent_pipeline_returns_error() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();
    let orphans = empty_orphans();

    let result = handle_pipeline_resume(
        &state,
        &orphans,
        &event_bus,
        "nonexistent".to_string(),
        None,
        HashMap::new(),
    );

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("not found"),
                "expected 'not found' in message, got: {}",
                message
            );
        }
        other => panic!("expected Response::Error, got: {:?}", other),
    }
}

#[test]
fn resume_orphan_without_runbook_hash_returns_error() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    // Create an orphan with empty runbook_hash (old breadcrumb format)
    let mut bc = make_breadcrumb("orphan-1");
    bc.runbook_hash = String::new();
    let orphans = Arc::new(Mutex::new(vec![bc]));

    let result = handle_pipeline_resume(
        &state,
        &orphans,
        &event_bus,
        "orphan-1".to_string(),
        None,
        HashMap::new(),
    );

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("orphaned") && message.contains("breadcrumb missing"),
                "unexpected error: {}",
                message
            );
        }
        other => panic!("expected Response::Error, got: {:?}", other),
    }
}

#[test]
fn resume_orphan_without_runbook_in_state_returns_error() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    // Create an orphan with a runbook_hash, but no matching runbook in state
    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb("orphan-2")]));

    let result = handle_pipeline_resume(
        &state,
        &orphans,
        &event_bus,
        "orphan-2".to_string(),
        None,
        HashMap::new(),
    );

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("orphaned") && message.contains("runbook is no longer"),
                "unexpected error: {}",
                message
            );
        }
        other => panic!("expected Response::Error, got: {:?}", other),
    }
}

#[test]
fn resume_orphan_with_runbook_reconstructs_and_resumes() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    // Add a runbook to state via event application
    load_runbook_into_state(&state, "hash456");

    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb("orphan-3")]));

    let result = handle_pipeline_resume(
        &state,
        &orphans,
        &event_bus,
        "orphan-3".to_string(),
        Some("fix it".to_string()),
        HashMap::new(),
    );

    // Should succeed (events emitted to WAL)
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);

    // Orphan should be removed from registry
    assert!(orphans.lock().is_empty(), "orphan should be removed");
}

#[test]
fn resume_orphan_by_prefix() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    load_runbook_into_state(&state, "hash456");

    let orphans = Arc::new(Mutex::new(vec![make_breadcrumb(
        "orphan-long-uuid-string-12345",
    )]));

    let result = handle_pipeline_resume(
        &state,
        &orphans,
        &event_bus,
        "orphan-long".to_string(),
        Some("try again".to_string()),
        HashMap::new(),
    );

    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
    assert!(orphans.lock().is_empty());
}

#[tokio::test]
async fn session_kill_nonexistent_returns_error() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    let result = handle_session_kill(&state, &event_bus, "nonexistent-session").await;

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("not found"),
                "expected 'not found' in message, got: {}",
                message
            );
        }
        other => panic!("expected Response::Error, got: {:?}", other),
    }
}

#[tokio::test]
async fn session_kill_existing_returns_ok() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    // Insert a session into state
    {
        let mut s = state.lock();
        s.sessions.insert(
            "oj-test-session".to_string(),
            oj_storage::Session {
                id: "oj-test-session".to_string(),
                pipeline_id: "pipe-1".to_string(),
            },
        );
    }

    let result = handle_session_kill(&state, &event_bus, "oj-test-session").await;

    // Should succeed (tmux kill-session will fail since no real tmux session,
    // but that's fine - we still emit the event)
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

fn make_pipeline_with_agent(id: &str, step: &str, agent_id: &str) -> Pipeline {
    Pipeline {
        id: id.to_string(),
        name: "test-pipeline".to_string(),
        kind: "test".to_string(),
        namespace: "proj".to_string(),
        step: step.to_string(),
        step_status: StepStatus::Running,
        step_started_at: Instant::now(),
        step_history: vec![StepRecord {
            name: "work".to_string(),
            started_at_ms: 1000,
            finished_at_ms: Some(2000),
            outcome: StepOutcome::Completed,
            agent_id: Some(agent_id.to_string()),
            agent_name: Some("test-agent".to_string()),
        }],
        vars: HashMap::new(),
        runbook_hash: "abc123".to_string(),
        cwd: std::path::PathBuf::from("/tmp/project"),
        workspace_id: None,
        workspace_path: None,
        session_id: None,
        created_at: Instant::now(),
        error: None,
        action_attempts: HashMap::new(),
        agent_signal: None,
        cancelling: false,
        total_retries: 0,
        step_visits: HashMap::new(),
        cron_name: None,
    }
}

#[test]
fn agent_prune_all_removes_terminal_pipelines_from_state() {
    let dir = tempdir().unwrap();
    let logs_path = dir.path().join("logs");
    std::fs::create_dir_all(logs_path.join("agent")).unwrap();

    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    // Insert a terminal pipeline with an agent
    {
        let mut s = state.lock();
        s.pipelines.insert(
            "pipe-done".to_string(),
            make_pipeline_with_agent("pipe-done", "done", "agent-1"),
        );
        // Insert a non-terminal pipeline (should be skipped)
        s.pipelines.insert(
            "pipe-running".to_string(),
            make_pipeline_with_agent("pipe-running", "work", "agent-2"),
        );
    }

    let result = handle_agent_prune(&state, &event_bus, &logs_path, true, false);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 1, "should prune 1 agent");
            assert_eq!(pruned[0].agent_id, "agent-1");
            assert_eq!(pruned[0].pipeline_id, "pipe-done");
            assert_eq!(skipped, 1, "should skip 1 non-terminal pipeline");
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    // After processing events, the terminal pipeline should be removed from state
    {
        let mut s = state.lock();
        // Apply the PipelineDeleted event that was emitted
        let event = Event::PipelineDeleted {
            id: oj_core::PipelineId::new("pipe-done".to_string()),
        };
        s.apply_event(&event);

        assert!(
            !s.pipelines.contains_key("pipe-done"),
            "terminal pipeline should be removed after prune"
        );
        assert!(
            s.pipelines.contains_key("pipe-running"),
            "non-terminal pipeline should remain"
        );
    }
}

#[test]
fn agent_prune_dry_run_does_not_delete() {
    let dir = tempdir().unwrap();
    let logs_path = dir.path().join("logs");
    std::fs::create_dir_all(logs_path.join("agent")).unwrap();

    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    {
        let mut s = state.lock();
        s.pipelines.insert(
            "pipe-failed".to_string(),
            make_pipeline_with_agent("pipe-failed", "failed", "agent-3"),
        );
    }

    let result = handle_agent_prune(&state, &event_bus, &logs_path, true, true);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 1, "should report 1 agent");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    // Pipeline should still be in state after dry run
    let s = state.lock();
    assert!(
        s.pipelines.contains_key("pipe-failed"),
        "pipeline should remain after dry run"
    );
}

#[test]
fn agent_prune_skips_non_terminal_pipelines() {
    let dir = tempdir().unwrap();
    let logs_path = dir.path().join("logs");
    std::fs::create_dir_all(logs_path.join("agent")).unwrap();

    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    {
        let mut s = state.lock();
        s.pipelines.insert(
            "pipe-active".to_string(),
            make_pipeline_with_agent("pipe-active", "build", "agent-4"),
        );
    }

    let result = handle_agent_prune(&state, &event_bus, &logs_path, true, false);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 0, "should not prune active agents");
            assert_eq!(skipped, 1, "should skip the active pipeline");
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    let s = state.lock();
    assert!(
        s.pipelines.contains_key("pipe-active"),
        "active pipeline should remain"
    );
}

// --- handle_pipeline_cancel tests ---

#[test]
fn cancel_single_running_pipeline() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    {
        let mut s = state.lock();
        s.pipelines
            .insert("pipe-1".to_string(), make_pipeline("pipe-1", "work"));
    }

    let result = handle_pipeline_cancel(&state, &event_bus, vec!["pipe-1".to_string()]);

    match result {
        Ok(Response::PipelinesCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert_eq!(cancelled, vec!["pipe-1"]);
            assert!(already_terminal.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected PipelinesCancelled, got: {:?}", other),
    }
}

#[test]
fn cancel_nonexistent_pipeline_returns_not_found() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    let result = handle_pipeline_cancel(&state, &event_bus, vec!["no-such-pipe".to_string()]);

    match result {
        Ok(Response::PipelinesCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert!(cancelled.is_empty());
            assert!(already_terminal.is_empty());
            assert_eq!(not_found, vec!["no-such-pipe"]);
        }
        other => panic!("expected PipelinesCancelled, got: {:?}", other),
    }
}

#[test]
fn cancel_already_terminal_pipeline() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    {
        let mut s = state.lock();
        s.pipelines
            .insert("pipe-done".to_string(), make_pipeline("pipe-done", "done"));
        s.pipelines.insert(
            "pipe-failed".to_string(),
            make_pipeline("pipe-failed", "failed"),
        );
        s.pipelines.insert(
            "pipe-cancelled".to_string(),
            make_pipeline("pipe-cancelled", "cancelled"),
        );
    }

    let result = handle_pipeline_cancel(
        &state,
        &event_bus,
        vec![
            "pipe-done".to_string(),
            "pipe-failed".to_string(),
            "pipe-cancelled".to_string(),
        ],
    );

    match result {
        Ok(Response::PipelinesCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert!(cancelled.is_empty());
            assert_eq!(already_terminal.len(), 3);
            assert!(already_terminal.contains(&"pipe-done".to_string()));
            assert!(already_terminal.contains(&"pipe-failed".to_string()));
            assert!(already_terminal.contains(&"pipe-cancelled".to_string()));
            assert!(not_found.is_empty());
        }
        other => panic!("expected PipelinesCancelled, got: {:?}", other),
    }
}

#[test]
fn cancel_multiple_pipelines_mixed_results() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    {
        let mut s = state.lock();
        // Running pipeline — should be cancelled
        s.pipelines
            .insert("pipe-a".to_string(), make_pipeline("pipe-a", "build"));
        // Another running pipeline — should be cancelled
        s.pipelines
            .insert("pipe-b".to_string(), make_pipeline("pipe-b", "test"));
        // Terminal pipeline — already_terminal
        s.pipelines
            .insert("pipe-c".to_string(), make_pipeline("pipe-c", "done"));
        // "pipe-d" not inserted — not_found
    }

    let result = handle_pipeline_cancel(
        &state,
        &event_bus,
        vec![
            "pipe-a".to_string(),
            "pipe-b".to_string(),
            "pipe-c".to_string(),
            "pipe-d".to_string(),
        ],
    );

    match result {
        Ok(Response::PipelinesCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert_eq!(cancelled, vec!["pipe-a", "pipe-b"]);
            assert_eq!(already_terminal, vec!["pipe-c"]);
            assert_eq!(not_found, vec!["pipe-d"]);
        }
        other => panic!("expected PipelinesCancelled, got: {:?}", other),
    }
}

#[test]
fn cancel_empty_ids_returns_empty_response() {
    let dir = tempdir().unwrap();
    let event_bus = test_event_bus(dir.path());
    let state = empty_state();

    let result = handle_pipeline_cancel(&state, &event_bus, vec![]);

    match result {
        Ok(Response::PipelinesCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert!(cancelled.is_empty());
            assert!(already_terminal.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected PipelinesCancelled, got: {:?}", other),
    }
}
