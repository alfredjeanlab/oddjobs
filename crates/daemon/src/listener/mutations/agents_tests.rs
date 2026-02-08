// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use tempfile::tempdir;

use oj_core::{AgentRunStatus, Event, StepOutcome, StepRecord};

use crate::protocol::Response;

use super::super::test_helpers::{
    make_agent_run, make_job, make_job_agent_in_history, make_job_with_agent, test_ctx,
};
use super::super::PruneFlags;
use super::{handle_agent_prune, handle_agent_send};

// --- handle_agent_prune tests ---

#[test]
fn agent_prune_all_removes_terminal_jobs_from_state() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-done".to_string(),
            make_job_with_agent("pipe-done", "done", "agent-1"),
        );
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

    {
        let mut s = ctx.state.lock();
        s.apply_event(&Event::JobDeleted {
            id: oj_core::JobId::new("pipe-done".to_string()),
        });
        assert!(!s.jobs.contains_key("pipe-done"));
        assert!(s.jobs.contains_key("pipe-running"));
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

    let s = ctx.state.lock();
    assert!(s.jobs.contains_key("pipe-failed"));
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
    assert!(s.jobs.contains_key("pipe-active"));
}

#[test]
fn agent_prune_all_removes_terminal_standalone_agent_runs() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

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
            for entry in &pruned {
                assert!(entry.job_id.is_empty(), "standalone agents have empty job_id");
            }
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    {
        let mut s = ctx.state.lock();
        s.apply_event(&Event::AgentRunDeleted {
            id: oj_core::AgentRunId::new("ar-completed"),
        });
        s.apply_event(&Event::AgentRunDeleted {
            id: oj_core::AgentRunId::new("ar-failed"),
        });
        assert!(!s.agent_runs.contains_key("ar-completed"));
        assert!(!s.agent_runs.contains_key("ar-failed"));
        assert!(s.agent_runs.contains_key("ar-running"));
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
            assert!(pruned[0].job_id.is_empty(), "standalone agent has empty job_id");
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    let s = ctx.state.lock();
    assert!(s.agent_runs.contains_key("ar-done"));
}

#[test]
fn agent_prune_all_handles_mixed_job_and_standalone_agents() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-done".to_string(),
            make_job_with_agent("pipe-done", "done", "agent-from-job"),
        );
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
            assert_eq!(pruned.len(), 2);
            assert_eq!(skipped, 0);
            assert!(pruned.iter().any(|e| !e.job_id.is_empty()));
            assert!(pruned.iter().any(|e| e.job_id.is_empty()));
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    {
        let mut s = ctx.state.lock();
        s.apply_event(&Event::JobDeleted {
            id: oj_core::JobId::new("pipe-done".to_string()),
        });
        s.apply_event(&Event::AgentRunDeleted {
            id: oj_core::AgentRunId::new("ar-done"),
        });
        assert!(!s.jobs.contains_key("pipe-done"));
        assert!(!s.agent_runs.contains_key("ar-done"));
    }
}

// --- handle_agent_send tests ---

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

    {
        let mut s = ctx.state.lock();
        s.jobs.insert(
            "pipe-abc123".to_string(),
            make_job_agent_in_history("pipe-abc123", "review", "work", "agent-inner"),
        );
    }

    let result = handle_agent_send(&ctx, "pipe-abc123".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[tokio::test]
async fn agent_send_prefix_match_across_all_history() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

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

    let result = handle_agent_send(&ctx, "pipe-multi".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}
