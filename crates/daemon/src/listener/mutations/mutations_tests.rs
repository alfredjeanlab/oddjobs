// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_core::{AgentRunStatus, Event, Job, StepOutcome, StepRecord, StepStatus, WorkspaceStatus};
use oj_engine::breadcrumb::Breadcrumb;
use oj_storage::{MaterializedState, Workspace, WorkspaceType};

use crate::protocol::Response;

use super::super::test_ctx;
use super::workspaces::workspace_prune_inner;
use super::{
    handle_agent_prune, handle_agent_send, handle_job_cancel, handle_job_prune, handle_job_resume,
    handle_job_resume_all, handle_session_kill, PruneFlags,
};

fn make_job(id: &str, step: &str) -> Job {
    Job::builder()
        .id(id)
        .kind("test")
        .namespace("proj")
        .step(step)
        .runbook_hash("abc123")
        .cwd("/tmp/project")
        .step_history(vec![StepRecord {
            name: step.to_string(),
            started_at_ms: 1000,
            finished_at_ms: None,
            outcome: StepOutcome::Running,
            agent_id: None,
            agent_name: None,
        }])
        .build()
}

fn make_breadcrumb(job_id: &str) -> Breadcrumb {
    Breadcrumb {
        job_id: job_id.to_string(),
        project: "proj".to_string(),
        kind: "test".to_string(),
        name: "test-job".to_string(),
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
fn resume_existing_job_emits_event() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Insert a job in state
    {
        let mut s = ctx.state.lock();
        s.jobs
            .insert("pipe-1".to_string(), make_job("pipe-1", "work"));
    }

    let result = handle_job_resume(
        &ctx,
        "pipe-1".to_string(),
        Some("try again".to_string()),
        HashMap::new(),
        false,
    );

    assert!(matches!(result, Ok(Response::Ok)));
}

#[test]
fn resume_nonexistent_job_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result = handle_job_resume(&ctx, "nonexistent".to_string(), None, HashMap::new(), false);

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
    let ctx = test_ctx(dir.path());

    // Create an orphan with empty runbook_hash (old breadcrumb format)
    let mut bc = make_breadcrumb("orphan-1");
    bc.runbook_hash = String::new();
    *ctx.orphans.lock() = vec![bc];

    let result = handle_job_resume(&ctx, "orphan-1".to_string(), None, HashMap::new(), false);

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
    let ctx = test_ctx(dir.path());

    // Create an orphan with a runbook_hash, but no matching runbook in state
    *ctx.orphans.lock() = vec![make_breadcrumb("orphan-2")];

    let result = handle_job_resume(&ctx, "orphan-2".to_string(), None, HashMap::new(), false);

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
    let ctx = test_ctx(dir.path());

    // Add a runbook to state via event application
    load_runbook_into_state(&ctx.state, "hash456");

    *ctx.orphans.lock() = vec![make_breadcrumb("orphan-3")];

    let result = handle_job_resume(
        &ctx,
        "orphan-3".to_string(),
        Some("fix it".to_string()),
        HashMap::new(),
        false,
    );

    // Should succeed (events emitted to WAL)
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);

    // Orphan should be removed from registry
    assert!(ctx.orphans.lock().is_empty(), "orphan should be removed");
}

#[test]
fn resume_orphan_by_prefix() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    load_runbook_into_state(&ctx.state, "hash456");

    *ctx.orphans.lock() = vec![make_breadcrumb("orphan-long-uuid-string-12345")];

    let result = handle_job_resume(
        &ctx,
        "orphan-long".to_string(),
        Some("try again".to_string()),
        HashMap::new(),
        false,
    );

    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
    assert!(ctx.orphans.lock().is_empty());
}

#[tokio::test]
async fn session_kill_nonexistent_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result = handle_session_kill(&ctx, "nonexistent-session").await;

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
    let ctx = test_ctx(dir.path());

    // Insert a session into state
    {
        let mut s = ctx.state.lock();
        s.sessions.insert(
            "oj-test-session".to_string(),
            oj_storage::Session {
                id: "oj-test-session".to_string(),
                job_id: "pipe-1".to_string(),
            },
        );
    }

    let result = handle_session_kill(&ctx, "oj-test-session").await;

    // Should succeed (tmux kill-session will fail since no real tmux session,
    // but that's fine - we still emit the event)
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

fn make_job_with_agent(id: &str, step: &str, agent_id: &str) -> Job {
    Job::builder()
        .id(id)
        .kind("test")
        .namespace("proj")
        .step(step)
        .runbook_hash("abc123")
        .cwd("/tmp/project")
        .step_history(vec![StepRecord {
            name: "work".to_string(),
            started_at_ms: 1000,
            finished_at_ms: Some(2000),
            outcome: StepOutcome::Completed,
            agent_id: Some(agent_id.to_string()),
            agent_name: Some("test-agent".to_string()),
        }])
        .build()
}

#[test]
fn agent_prune_all_removes_terminal_jobs_from_state() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    // Insert a terminal job with an agent
    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-done".to_string(),
            make_job_with_agent("pipe-done", "done", "agent-1"),
        );
        // Insert a non-terminal job (should be skipped)
        s.jobs.insert(
            "pipe-running".to_string(),
            make_job_with_agent("pipe-running", "work", "agent-2"),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: None,
    };
    let result = handle_agent_prune(&ctx, &flags);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 1, "should prune 1 agent");
            assert_eq!(pruned[0].agent_id, "agent-1");
            assert_eq!(pruned[0].job_id, "pipe-done");
            assert_eq!(skipped, 1, "should skip 1 non-terminal job");
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    // After processing events, the terminal job should be removed from state
    {
        let mut s = ctx.state.lock();
        // Apply the JobDeleted event that was emitted
        let event = Event::JobDeleted {
            id: oj_core::JobId::new("pipe-done".to_string()),
        };
        s.apply_event(&event);

        assert!(
            !s.jobs.contains_key("pipe-done"),
            "terminal job should be removed after prune"
        );
        assert!(
            s.jobs.contains_key("pipe-running"),
            "non-terminal job should remain"
        );
    }
}

#[test]
fn agent_prune_dry_run_does_not_delete() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-failed".to_string(),
            make_job_with_agent("pipe-failed", "failed", "agent-3"),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: true,
        namespace: None,
    };
    let result = handle_agent_prune(&ctx, &flags);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 1, "should report 1 agent");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    // Job should still be in state after dry run
    let s = ctx.state.lock();
    assert!(
        s.jobs.contains_key("pipe-failed"),
        "job should remain after dry run"
    );
}

#[test]
fn agent_prune_skips_non_terminal_jobs() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-active".to_string(),
            make_job_with_agent("pipe-active", "build", "agent-4"),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: None,
    };
    let result = handle_agent_prune(&ctx, &flags);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 0, "should not prune active agents");
            assert_eq!(skipped, 1, "should skip the active job");
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    let s = ctx.state.lock();
    assert!(
        s.jobs.contains_key("pipe-active"),
        "active job should remain"
    );
}

fn make_agent_run(id: &str, status: AgentRunStatus) -> oj_core::AgentRun {
    oj_core::AgentRun::builder()
        .id(id)
        .agent_name("test-agent")
        .command_name("test-cmd")
        .namespace("proj")
        .cwd("/tmp/project")
        .runbook_hash("hash123")
        .status(status)
        .agent_id(format!("{}-agent-uuid", id))
        .session_id(format!("oj-{}", id))
        .build()
}

#[test]
fn agent_prune_all_removes_terminal_standalone_agent_runs() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    // Insert terminal and non-terminal standalone agent runs
    {
        let mut s = ctx.state.lock();
        s.agent_runs.insert(
            "ar-completed".to_string(),
            make_agent_run("ar-completed", AgentRunStatus::Completed),
        );
        s.agent_runs.insert(
            "ar-failed".to_string(),
            make_agent_run("ar-failed", AgentRunStatus::Failed),
        );
        s.agent_runs.insert(
            "ar-running".to_string(),
            make_agent_run("ar-running", AgentRunStatus::Running),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: None,
    };
    let result = handle_agent_prune(&ctx, &flags);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 2, "should prune 2 terminal agent runs");
            assert_eq!(skipped, 1, "should skip 1 running agent run");

            // Verify pruned entries have empty job_id (standalone)
            for entry in &pruned {
                assert!(
                    entry.job_id.is_empty(),
                    "standalone agents have empty job_id"
                );
            }
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    // Apply the AgentRunDeleted events to state and verify
    {
        let mut s = ctx.state.lock();
        s.apply_event(&Event::AgentRunDeleted {
            id: oj_core::AgentRunId::new("ar-completed"),
        });
        s.apply_event(&Event::AgentRunDeleted {
            id: oj_core::AgentRunId::new("ar-failed"),
        });

        assert!(
            !s.agent_runs.contains_key("ar-completed"),
            "completed should be pruned"
        );
        assert!(
            !s.agent_runs.contains_key("ar-failed"),
            "failed should be pruned"
        );
        assert!(
            s.agent_runs.contains_key("ar-running"),
            "running should remain"
        );
    }
}

#[test]
fn agent_prune_dry_run_does_not_delete_standalone_agent_runs() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    {
        let mut s = ctx.state.lock();
        s.agent_runs.insert(
            "ar-done".to_string(),
            make_agent_run("ar-done", AgentRunStatus::Completed),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: true,
        namespace: None,
    };
    let result = handle_agent_prune(&ctx, &flags);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 1, "should report 1 agent");
            assert_eq!(skipped, 0);
            // Verify it's a standalone agent entry
            assert!(
                pruned[0].job_id.is_empty(),
                "standalone agent has empty job_id"
            );
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    // Verify agent run was NOT deleted (dry run - no events emitted)
    let s = ctx.state.lock();
    assert!(
        s.agent_runs.contains_key("ar-done"),
        "dry run should not delete"
    );
}

#[test]
fn agent_prune_all_handles_mixed_job_and_standalone_agents() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    {
        let mut s = ctx.state.lock();
        // Terminal job with agent
        s.jobs.insert(
            "pipe-done".to_string(),
            make_job_with_agent("pipe-done", "done", "agent-from-job"),
        );
        // Terminal standalone agent run
        s.agent_runs.insert(
            "ar-done".to_string(),
            make_agent_run("ar-done", AgentRunStatus::Completed),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: None,
    };
    let result = handle_agent_prune(&ctx, &flags);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(
                pruned.len(),
                2,
                "should prune both job agent and standalone"
            );
            assert_eq!(skipped, 0);

            // Find the entries
            let job_agent = pruned.iter().find(|e| !e.job_id.is_empty());
            let standalone_agent = pruned.iter().find(|e| e.job_id.is_empty());

            assert!(job_agent.is_some(), "should have job agent");
            assert!(standalone_agent.is_some(), "should have standalone agent");
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    // Apply the emitted events and verify state
    {
        let mut s = ctx.state.lock();
        s.apply_event(&Event::JobDeleted {
            id: oj_core::JobId::new("pipe-done".to_string()),
        });
        s.apply_event(&Event::AgentRunDeleted {
            id: oj_core::AgentRunId::new("ar-done"),
        });

        assert!(!s.jobs.contains_key("pipe-done"), "job should be pruned");
        assert!(
            !s.agent_runs.contains_key("ar-done"),
            "agent run should be pruned"
        );
    }
}

// --- cleanup helper tests ---

#[test]
fn cleanup_job_files_removes_log_and_breadcrumb() {
    let dir = tempdir().unwrap();
    let logs_path = dir.path().join("logs");
    std::fs::create_dir_all(logs_path.join("agent")).unwrap();

    // Create job log, breadcrumb, and agent files
    let log_file = oj_engine::log_paths::job_log_path(&logs_path, "pipe-cleanup");
    if let Some(parent) = log_file.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&log_file, "log data").unwrap();

    let crumb_file = oj_engine::log_paths::breadcrumb_path(&logs_path, "pipe-cleanup");
    if let Some(parent) = crumb_file.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&crumb_file, "crumb data").unwrap();

    let agent_log = logs_path.join("agent").join("pipe-cleanup.log");
    std::fs::write(&agent_log, "agent log").unwrap();

    let agent_dir = logs_path.join("agent").join("pipe-cleanup");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("session.log"), "session").unwrap();

    super::prune_helpers::cleanup_job_files(&logs_path, "pipe-cleanup");

    assert!(!log_file.exists(), "job log should be removed");
    assert!(!crumb_file.exists(), "breadcrumb should be removed");
    assert!(!agent_log.exists(), "agent log should be removed");
    assert!(!agent_dir.exists(), "agent dir should be removed");
}

#[test]
fn cleanup_agent_files_removes_log_and_dir() {
    let dir = tempdir().unwrap();
    let logs_path = dir.path().join("logs");
    std::fs::create_dir_all(logs_path.join("agent")).unwrap();

    let agent_log = logs_path.join("agent").join("agent-42.log");
    std::fs::write(&agent_log, "data").unwrap();

    let agent_dir = logs_path.join("agent").join("agent-42");
    std::fs::create_dir_all(&agent_dir).unwrap();

    super::prune_helpers::cleanup_agent_files(&logs_path, "agent-42");

    assert!(!agent_log.exists(), "agent log should be removed");
    assert!(!agent_dir.exists(), "agent dir should be removed");
}

// --- handle_job_cancel tests ---

#[test]
fn cancel_single_running_job() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        s.jobs
            .insert("pipe-1".to_string(), make_job("pipe-1", "work"));
    }

    let result = handle_job_cancel(&ctx, vec!["pipe-1".to_string()]);

    match result {
        Ok(Response::JobsCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert_eq!(cancelled, vec!["pipe-1"]);
            assert!(already_terminal.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected JobsCancelled, got: {:?}", other),
    }
}

#[test]
fn cancel_nonexistent_job_returns_not_found() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result = handle_job_cancel(&ctx, vec!["no-such-pipe".to_string()]);

    match result {
        Ok(Response::JobsCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert!(cancelled.is_empty());
            assert!(already_terminal.is_empty());
            assert_eq!(not_found, vec!["no-such-pipe"]);
        }
        other => panic!("expected JobsCancelled, got: {:?}", other),
    }
}

#[test]
fn cancel_already_terminal_job() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        s.jobs
            .insert("pipe-done".to_string(), make_job("pipe-done", "done"));
        s.jobs
            .insert("pipe-failed".to_string(), make_job("pipe-failed", "failed"));
        s.jobs.insert(
            "pipe-cancelled".to_string(),
            make_job("pipe-cancelled", "cancelled"),
        );
    }

    let result = handle_job_cancel(
        &ctx,
        vec![
            "pipe-done".to_string(),
            "pipe-failed".to_string(),
            "pipe-cancelled".to_string(),
        ],
    );

    match result {
        Ok(Response::JobsCancelled {
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
        other => panic!("expected JobsCancelled, got: {:?}", other),
    }
}

#[test]
fn cancel_multiple_jobs_mixed_results() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        // Running job — should be cancelled
        s.jobs
            .insert("pipe-a".to_string(), make_job("pipe-a", "build"));
        // Another running job — should be cancelled
        s.jobs
            .insert("pipe-b".to_string(), make_job("pipe-b", "test"));
        // Terminal job — already_terminal
        s.jobs
            .insert("pipe-c".to_string(), make_job("pipe-c", "done"));
        // "pipe-d" not inserted — not_found
    }

    let result = handle_job_cancel(
        &ctx,
        vec![
            "pipe-a".to_string(),
            "pipe-b".to_string(),
            "pipe-c".to_string(),
            "pipe-d".to_string(),
        ],
    );

    match result {
        Ok(Response::JobsCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert_eq!(cancelled, vec!["pipe-a", "pipe-b"]);
            assert_eq!(already_terminal, vec!["pipe-c"]);
            assert_eq!(not_found, vec!["pipe-d"]);
        }
        other => panic!("expected JobsCancelled, got: {:?}", other),
    }
}

#[test]
fn cancel_empty_ids_returns_empty_response() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result = handle_job_cancel(&ctx, vec![]);

    match result {
        Ok(Response::JobsCancelled {
            cancelled,
            already_terminal,
            not_found,
        }) => {
            assert!(cancelled.is_empty());
            assert!(already_terminal.is_empty());
            assert!(not_found.is_empty());
        }
        other => panic!("expected JobsCancelled, got: {:?}", other),
    }
}

/// Helper to create a runbook JSON with an agent step
fn make_agent_runbook_json(job_kind: &str, step_name: &str) -> serde_json::Value {
    serde_json::json!({
        "jobs": {
            job_kind: {
                "kind": job_kind,
                "steps": [
                    {
                        "name": step_name,
                        "run": { "agent": "test-agent" }
                    }
                ]
            }
        }
    })
}

/// Helper to create a runbook JSON with a shell step
fn make_shell_runbook_json(job_kind: &str, step_name: &str) -> serde_json::Value {
    serde_json::json!({
        "jobs": {
            job_kind: {
                "kind": job_kind,
                "steps": [
                    {
                        "name": step_name,
                        "run": "echo hello"
                    }
                ]
            }
        }
    })
}

/// Load a runbook JSON into state with a specific hash
fn load_runbook_json_into_state(
    state: &Arc<Mutex<MaterializedState>>,
    hash: &str,
    runbook_json: serde_json::Value,
) {
    let event = Event::RunbookLoaded {
        hash: hash.to_string(),
        version: 1,
        runbook: runbook_json,
    };
    state.lock().apply_event(&event);
}

#[test]
fn resume_agent_step_without_message_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Create a runbook with an agent step
    let runbook_hash = "agent-runbook-hash";
    load_runbook_json_into_state(
        &ctx.state,
        runbook_hash,
        make_agent_runbook_json("test", "work"),
    );

    // Create a job at the agent step
    let mut job = make_job("pipe-agent", "work");
    job.runbook_hash = runbook_hash.to_string();
    {
        let mut s = ctx.state.lock();
        s.jobs.insert("pipe-agent".to_string(), job);
    }

    // Try to resume without a message
    let result = handle_job_resume(
        &ctx,
        "pipe-agent".to_string(),
        None, // No message provided
        HashMap::new(),
        false,
    );

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("--message") || message.contains("agent steps require"),
                "expected error about --message, got: {}",
                message
            );
        }
        other => panic!("expected Response::Error about --message, got: {:?}", other),
    }
}

#[test]
fn resume_agent_step_with_message_succeeds() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Create a runbook with an agent step
    let runbook_hash = "agent-runbook-hash";
    load_runbook_json_into_state(
        &ctx.state,
        runbook_hash,
        make_agent_runbook_json("test", "work"),
    );

    // Create a job at the agent step
    let mut job = make_job("pipe-agent-2", "work");
    job.runbook_hash = runbook_hash.to_string();
    {
        let mut s = ctx.state.lock();
        s.jobs.insert("pipe-agent-2".to_string(), job);
    }

    // Resume with a message should succeed
    let result = handle_job_resume(
        &ctx,
        "pipe-agent-2".to_string(),
        Some("I fixed the issue".to_string()),
        HashMap::new(),
        false,
    );

    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[test]
fn resume_shell_step_without_message_succeeds() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Create a runbook with a shell step
    let runbook_hash = "shell-runbook-hash";
    load_runbook_json_into_state(
        &ctx.state,
        runbook_hash,
        make_shell_runbook_json("test", "build"),
    );

    // Create a job at the shell step
    let mut job = make_job("pipe-shell", "build");
    job.runbook_hash = runbook_hash.to_string();
    {
        let mut s = ctx.state.lock();
        s.jobs.insert("pipe-shell".to_string(), job);
    }

    // Resume without a message should succeed for shell steps
    let result = handle_job_resume(&ctx, "pipe-shell".to_string(), None, HashMap::new(), false);

    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[test]
fn resume_failed_job_without_message_succeeds() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Create a runbook with an agent step
    let runbook_hash = "agent-runbook-hash";
    load_runbook_json_into_state(
        &ctx.state,
        runbook_hash,
        make_agent_runbook_json("test", "work"),
    );

    // Create a job in "failed" state (terminal failure)
    // Even though the last step was an agent step, resuming from "failed"
    // doesn't require a message at the daemon level - the engine handles
    // resetting to the failed step
    let mut job = make_job("pipe-failed-agent", "failed");
    job.runbook_hash = runbook_hash.to_string();
    {
        let mut s = ctx.state.lock();
        s.jobs.insert("pipe-failed-agent".to_string(), job);
    }

    // Resume without message should be allowed for "failed" state
    // (the engine will reset to the actual failed step and validate there)
    let result = handle_job_resume(
        &ctx,
        "pipe-failed-agent".to_string(),
        None,
        HashMap::new(),
        false,
    );

    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

// --- handle_agent_send tests ---

/// Helper: build a job where the agent step is NOT the last step.
/// This simulates a job that has advanced past the agent step.
fn make_job_agent_in_history(
    id: &str,
    current_step: &str,
    agent_step: &str,
    agent_id: &str,
) -> Job {
    Job::builder()
        .id(id)
        .kind("test")
        .namespace("proj")
        .step(current_step)
        .runbook_hash("abc123")
        .cwd("/tmp/project")
        .step_history(vec![
            StepRecord {
                name: agent_step.to_string(),
                started_at_ms: 1000,
                finished_at_ms: Some(2000),
                outcome: StepOutcome::Completed,
                agent_id: Some(agent_id.to_string()),
                agent_name: Some("test-agent".to_string()),
            },
            StepRecord {
                name: current_step.to_string(),
                started_at_ms: 2000,
                finished_at_ms: None,
                outcome: StepOutcome::Running,
                agent_id: None,
                agent_name: None,
            },
        ])
        .build()
}

#[tokio::test]
async fn agent_send_finds_agent_in_last_step() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-1".to_string(),
            make_job_with_agent("pipe-1", "work", "agent-abc"),
        );
    }

    let result = handle_agent_send(&ctx, "agent-abc".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[tokio::test]
async fn agent_send_finds_agent_in_earlier_step() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Agent step is NOT the last step — job has advanced to "review"
    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-1".to_string(),
            make_job_agent_in_history("pipe-1", "review", "work", "agent-xyz"),
        );
    }

    let result = handle_agent_send(&ctx, "agent-xyz".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[tokio::test]
async fn agent_send_via_job_id_finds_agent_in_earlier_step() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Job has advanced past the agent step
    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-abc123".to_string(),
            make_job_agent_in_history("pipe-abc123", "review", "work", "agent-inner"),
        );
    }

    // Look up by job ID — should search all history and find the agent
    let result = handle_agent_send(&ctx, "pipe-abc123".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[tokio::test]
async fn agent_send_prefix_match_across_all_history() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Agent ID in a non-last step, matched by prefix
    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-1".to_string(),
            make_job_agent_in_history("pipe-1", "review", "work", "agent-long-uuid-string-12345"),
        );
    }

    let result = handle_agent_send(&ctx, "agent-long".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[tokio::test]
async fn agent_send_finds_standalone_agent_run() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Insert a standalone agent run (no job)
    {
        let mut s = ctx.state.lock();
        s.agent_runs.insert(
            "run-1".to_string(),
            oj_core::AgentRun::builder()
                .id("run-1")
                .agent_name("my-agent")
                .command_name("oj agent run")
                .namespace("proj")
                .cwd("/tmp")
                .runbook_hash("hash")
                .agent_id("standalone-agent-42")
                .session_id("oj-standalone-42")
                .build(),
        );
    }

    let result =
        handle_agent_send(&ctx, "standalone-agent-42".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[tokio::test]
async fn agent_send_not_found_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result =
        handle_agent_send(&ctx, "nonexistent-agent".to_string(), "hello".to_string()).await;

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
async fn agent_send_prefers_latest_step_history_entry() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    // Job with two agent steps — should prefer the latest (second) one
    // when looking up by job ID
    {
        let mut s = ctx.state.lock();
        let mut job = make_job("pipe-multi", "done");
        job.step_history = vec![
            StepRecord {
                name: "work-1".to_string(),
                started_at_ms: 1000,
                finished_at_ms: Some(2000),
                outcome: StepOutcome::Completed,
                agent_id: Some("agent-old".to_string()),
                agent_name: Some("agent-v1".to_string()),
            },
            StepRecord {
                name: "work-2".to_string(),
                started_at_ms: 2000,
                finished_at_ms: Some(3000),
                outcome: StepOutcome::Completed,
                agent_id: Some("agent-new".to_string()),
                agent_name: Some("agent-v2".to_string()),
            },
            StepRecord {
                name: "done".to_string(),
                started_at_ms: 3000,
                finished_at_ms: None,
                outcome: StepOutcome::Running,
                agent_id: None,
                agent_name: None,
            },
        ];
        s.jobs.insert("pipe-multi".to_string(), job);
    }

    // Look up by job ID — should resolve to the latest agent (agent-new)
    let result = handle_agent_send(&ctx, "pipe-multi".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

// --- handle_job_prune tests ---

fn make_job_ns(id: &str, step: &str, namespace: &str) -> Job {
    let mut p = make_job(id, step);
    p.namespace = namespace.to_string();
    p
}

#[test]
fn job_prune_all_without_namespace_prunes_across_all_projects() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(&ctx.logs_path).unwrap();

    // Insert terminal jobs from different namespaces
    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-a".to_string(),
            make_job_ns("pipe-a", "done", "proj-alpha"),
        );
        s.jobs.insert(
            "pipe-b".to_string(),
            make_job_ns("pipe-b", "failed", "proj-beta"),
        );
        s.jobs.insert(
            "pipe-c".to_string(),
            make_job_ns("pipe-c", "cancelled", "proj-gamma"),
        );
        // Non-terminal job should be skipped
        s.jobs.insert(
            "pipe-d".to_string(),
            make_job_ns("pipe-d", "work", "proj-alpha"),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: None, // No namespace filter
    };
    let result = handle_job_prune(&ctx, &flags, false, false);

    match result {
        Ok(Response::JobsPruned { pruned, skipped }) => {
            assert_eq!(
                pruned.len(),
                3,
                "should prune all 3 terminal jobs across namespaces"
            );
            let pruned_ids: Vec<&str> = pruned.iter().map(|e| e.id.as_str()).collect();
            assert!(pruned_ids.contains(&"pipe-a"));
            assert!(pruned_ids.contains(&"pipe-b"));
            assert!(pruned_ids.contains(&"pipe-c"));
            assert_eq!(skipped, 1, "should skip non-terminal job");
        }
        other => panic!("expected JobsPruned, got: {:?}", other),
    }
}

#[test]
fn job_prune_all_with_namespace_only_prunes_matching_project() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(&ctx.logs_path).unwrap();

    // Insert terminal jobs from different namespaces
    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-a".to_string(),
            make_job_ns("pipe-a", "done", "proj-alpha"),
        );
        s.jobs.insert(
            "pipe-b".to_string(),
            make_job_ns("pipe-b", "failed", "proj-beta"),
        );
        s.jobs.insert(
            "pipe-c".to_string(),
            make_job_ns("pipe-c", "cancelled", "proj-alpha"),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: Some("proj-alpha"), // Only prune proj-alpha
    };
    let result = handle_job_prune(&ctx, &flags, false, false);

    match result {
        Ok(Response::JobsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 2, "should prune only proj-alpha jobs");
            let pruned_ids: Vec<&str> = pruned.iter().map(|e| e.id.as_str()).collect();
            assert!(pruned_ids.contains(&"pipe-a"));
            assert!(pruned_ids.contains(&"pipe-c"));
            // Namespace-filtered jobs don't count as "skipped" —
            // only non-terminal jobs within the namespace do.
            assert_eq!(skipped, 0);
        }
        other => panic!("expected JobsPruned, got: {:?}", other),
    }
}

#[test]
fn job_prune_skips_non_terminal_steps() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(&ctx.logs_path).unwrap();

    {
        let mut s = ctx.state.lock();
        // Non-terminal steps should never be pruned
        s.jobs.insert(
            "pipe-running".to_string(),
            make_job("pipe-running", "implement"),
        );
        s.jobs
            .insert("pipe-work".to_string(), make_job("pipe-work", "work"));
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: None,
    };
    let result = handle_job_prune(&ctx, &flags, false, false);

    match result {
        Ok(Response::JobsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 0, "should not prune non-terminal jobs");
            assert_eq!(skipped, 2, "should skip both non-terminal jobs");
        }
        other => panic!("expected JobsPruned, got: {:?}", other),
    }
}

// --- handle_workspace_prune tests ---

fn make_workspace(id: &str, path: std::path::PathBuf, owner: Option<&str>) -> Workspace {
    Workspace {
        id: id.to_string(),
        path,
        branch: None,
        owner: owner.map(|o| oj_core::OwnerId::Job(oj_core::JobId::new(o))),
        status: WorkspaceStatus::Ready,
        workspace_type: WorkspaceType::default(),
        created_at_ms: 0,
    }
}

#[tokio::test]
async fn workspace_prune_emits_deleted_events_for_fs_workspaces() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    // Create workspace directories on the filesystem
    let ws1_path = workspaces_dir.join("ws-test-1");
    let ws2_path = workspaces_dir.join("ws-test-2");
    std::fs::create_dir_all(&ws1_path).unwrap();
    std::fs::create_dir_all(&ws2_path).unwrap();

    // Add workspace entries to daemon state
    {
        let mut s = ctx.state.lock();
        s.workspaces.insert(
            "ws-test-1".to_string(),
            make_workspace("ws-test-1", ws1_path.clone(), None),
        );
        s.workspaces.insert(
            "ws-test-2".to_string(),
            make_workspace("ws-test-2", ws2_path.clone(), None),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: None,
    };
    let result = workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir).await;

    match result {
        Ok(Response::WorkspacesPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 2, "should prune both workspaces");
            assert_eq!(skipped, 0);
            let ids: Vec<&str> = pruned.iter().map(|ws| ws.id.as_str()).collect();
            assert!(ids.contains(&"ws-test-1"));
            assert!(ids.contains(&"ws-test-2"));
        }
        other => panic!("expected WorkspacesPruned, got: {:?}", other),
    }

    // Directories should be removed
    assert!(!ws1_path.exists(), "ws-test-1 directory should be removed");
    assert!(!ws2_path.exists(), "ws-test-2 directory should be removed");

    // Verify WorkspaceDeleted events were emitted by applying them to state
    {
        let mut s = ctx.state.lock();
        s.apply_event(&Event::WorkspaceDeleted {
            id: oj_core::WorkspaceId::new("ws-test-1"),
        });
        s.apply_event(&Event::WorkspaceDeleted {
            id: oj_core::WorkspaceId::new("ws-test-2"),
        });
        assert!(!s.workspaces.contains_key("ws-test-1"));
        assert!(!s.workspaces.contains_key("ws-test-2"));
    }
}

#[tokio::test]
async fn workspace_prune_removes_orphaned_state_entries() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    // Add workspace entries to state that have NO corresponding filesystem directory
    {
        let mut s = ctx.state.lock();
        s.workspaces.insert(
            "ws-orphan-1".to_string(),
            make_workspace("ws-orphan-1", workspaces_dir.join("ws-orphan-1"), None),
        );
        s.workspaces.insert(
            "ws-orphan-2".to_string(),
            make_workspace("ws-orphan-2", workspaces_dir.join("ws-orphan-2"), None),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: None,
    };
    let result = workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir).await;

    match result {
        Ok(Response::WorkspacesPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 2, "should prune orphaned state entries");
            assert_eq!(skipped, 0);
            let ids: Vec<&str> = pruned.iter().map(|ws| ws.id.as_str()).collect();
            assert!(ids.contains(&"ws-orphan-1"));
            assert!(ids.contains(&"ws-orphan-2"));
        }
        other => panic!("expected WorkspacesPruned, got: {:?}", other),
    }
}

#[tokio::test]
async fn workspace_prune_dry_run_does_not_delete() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    let ws_path = workspaces_dir.join("ws-keep");
    std::fs::create_dir_all(&ws_path).unwrap();

    {
        let mut s = ctx.state.lock();
        s.workspaces.insert(
            "ws-keep".to_string(),
            make_workspace("ws-keep", ws_path.clone(), None),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: true,
        namespace: None,
    };
    let result = workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir).await;

    match result {
        Ok(Response::WorkspacesPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 1, "should report 1 workspace");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected WorkspacesPruned, got: {:?}", other),
    }

    // Directory should still exist after dry run
    assert!(
        ws_path.exists(),
        "workspace dir should remain after dry run"
    );

    // State should be unchanged after dry run
    let s = ctx.state.lock();
    assert!(
        s.workspaces.contains_key("ws-keep"),
        "workspace should remain in state after dry run"
    );
}

#[tokio::test]
async fn workspace_prune_includes_orphaned_owner_workspaces_with_namespace() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    // Workspace with an owner whose job no longer exists in state
    // (owner is unresolvable → should be included in namespace-filtered prune)
    {
        let mut s = ctx.state.lock();
        s.workspaces.insert(
            "ws-orphan-owner".to_string(),
            make_workspace(
                "ws-orphan-owner",
                workspaces_dir.join("ws-orphan-owner"),
                Some("deleted-job-id"),
            ),
        );
        // Workspace with a matching namespace job
        s.jobs.insert(
            "live-job".to_string(),
            make_job_ns("live-job", "done", "myproject"),
        );
        s.workspaces.insert(
            "ws-with-owner".to_string(),
            make_workspace(
                "ws-with-owner",
                workspaces_dir.join("ws-with-owner"),
                Some("live-job"),
            ),
        );
    }

    let flags = PruneFlags {
        all: true,
        dry_run: false,
        namespace: Some("myproject"),
    };
    let result = workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir).await;

    match result {
        Ok(Response::WorkspacesPruned { pruned, .. }) => {
            let ids: Vec<&str> = pruned.iter().map(|ws| ws.id.as_str()).collect();
            // Both should be pruned: orphaned owner is included, matching namespace is included
            assert!(
                ids.contains(&"ws-orphan-owner"),
                "orphaned owner workspace should be pruned, got: {:?}",
                ids
            );
            assert!(
                ids.contains(&"ws-with-owner"),
                "matching namespace workspace should be pruned, got: {:?}",
                ids
            );
        }
        other => panic!("expected WorkspacesPruned, got: {:?}", other),
    }
}

// -- Job Resume All ---------------------------------------------------------

#[test]
fn resume_all_resumes_waiting_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let mut job = make_job("job-1", "work");
        job.step_status = StepStatus::Waiting(None);
        s.jobs.insert("job-1".to_string(), job);
    }

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert_eq!(resumed, vec!["job-1"]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_resumes_failed_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let mut job = make_job("job-1", "work");
        job.step_status = StepStatus::Failed;
        s.jobs.insert("job-1".to_string(), job);
    }

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert_eq!(resumed, vec!["job-1"]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_skips_running_jobs_without_kill() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let job = make_job("job-1", "work"); // step_status = Running by default
        s.jobs.insert("job-1".to_string(), job);
    }

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert!(resumed.is_empty());
            assert_eq!(skipped.len(), 1);
            assert_eq!(skipped[0].0, "job-1");
            assert!(skipped[0].1.contains("--kill"));
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_with_kill_resumes_running_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let job = make_job("job-1", "work"); // step_status = Running
        s.jobs.insert("job-1".to_string(), job);
    }

    let result = handle_job_resume_all(&ctx, true);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert_eq!(resumed, vec!["job-1"]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_skips_terminal_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        // Terminal job (done)
        let done_job = make_job("job-done", "done");
        s.jobs.insert("job-done".to_string(), done_job);

        // Non-terminal waiting job
        let mut waiting_job = make_job("job-wait", "work");
        waiting_job.step_status = StepStatus::Waiting(None);
        s.jobs.insert("job-wait".to_string(), waiting_job);
    }

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert_eq!(resumed, vec!["job-wait"]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_returns_empty_when_no_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert!(resumed.is_empty());
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}
