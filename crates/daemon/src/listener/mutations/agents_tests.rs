// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use tempfile::tempdir;

use oj_core::{CrewStatus, StepOutcome, StepRecord};

use crate::protocol::Response;

use super::super::PruneFlags;
use super::{handle_agent_prune, handle_agent_send};
use crate::listener::test_ctx;
use crate::listener::test_fixtures::{
    make_crew, make_job, make_job_agent_in_history, make_job_with_agent,
};

// --- handle_agent_prune tests ---

#[yare::parameterized(
    removes_terminal_jobs       = { 1, 1, 0, 0, false, 1, 1 },
    dry_run_terminal_job        = { 1, 0, 0, 0, true,  1, 0 },
    skips_active_jobs           = { 0, 1, 0, 0, false, 0, 1 },
    removes_terminal_standalone = { 0, 0, 2, 1, false, 2, 1 },
    dry_run_standalone          = { 0, 0, 1, 0, true,  1, 0 },
    mixed_job_and_standalone    = { 1, 0, 1, 0, false, 2, 0 },
)]
fn agent_prune(
    terminal_jobs: usize,
    active_jobs: usize,
    terminal_runs: usize,
    running_runs: usize,
    dry_run: bool,
    expected_pruned: usize,
    expected_skipped: usize,
) {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(ctx.logs_path.join("agent")).unwrap();

    {
        let mut s = ctx.state.lock();
        for i in 0..terminal_jobs {
            s.jobs.insert(
                format!("job-done-{i}"),
                make_job_with_agent(&format!("job-done-{i}"), "done", &format!("agent-t{i}")),
            );
        }
        for i in 0..active_jobs {
            s.jobs.insert(
                format!("job-active-{i}"),
                make_job_with_agent(&format!("job-active-{i}"), "work", &format!("agent-a{i}")),
            );
        }
        for i in 0..terminal_runs {
            let status = if i % 2 == 0 { CrewStatus::Completed } else { CrewStatus::Failed };
            s.crew.insert(format!("run-done-{i}"), make_crew(&format!("run-done-{i}"), status));
        }
        for i in 0..running_runs {
            s.crew.insert(
                format!("run-running-{i}"),
                make_crew(&format!("run-running-{i}"), CrewStatus::Running),
            );
        }
    }

    let flags = PruneFlags { all: true, dry_run, project: None };
    let result = handle_agent_prune(&ctx, &flags);

    match result {
        Ok(Response::AgentsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), expected_pruned, "pruned count");
            assert_eq!(skipped, expected_skipped, "skipped count");
        }
        other => panic!("expected AgentsPruned, got: {:?}", other),
    }

    // Verify non-terminal entities remain in state
    let s = ctx.state.lock();
    for i in 0..active_jobs {
        assert!(s.jobs.contains_key(&format!("job-active-{i}")));
    }
    for i in 0..running_runs {
        assert!(s.crew.contains_key(&format!("run-running-{i}")));
    }
    if dry_run {
        for i in 0..terminal_jobs {
            assert!(s.jobs.contains_key(&format!("job-done-{i}")));
        }
        for i in 0..terminal_runs {
            assert!(s.crew.contains_key(&format!("run-done-{i}")));
        }
    }
}

// --- handle_agent_send tests ---

#[tokio::test]
async fn agent_send_finds_agent_in_last_step() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        s.jobs.insert("job-1".to_string(), make_job_with_agent("job-1", "work", "agent-abc"));
    }

    let result = handle_agent_send(&ctx, "agent-abc".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

#[yare::parameterized(
    by_agent_id = { "job-1",      "review", "work", "agent-xyz",                    "agent-xyz" },
    by_job_id   = { "job-abc123", "review", "work", "agent-inner",                  "job-abc123" },
    by_prefix   = { "job-1",      "review", "work", "agt-long_uuid_string12", "agt-long" },
)]
fn agent_send_lookup(
    job_id: &str,
    current_step: &str,
    agent_step: &str,
    agent_id: &str,
    query: &str,
) {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let dir = tempdir().unwrap();
        let ctx = test_ctx(dir.path());

        {
            let mut s = ctx.state.lock();
            s.jobs.insert(
                job_id.to_string(),
                make_job_agent_in_history(job_id, current_step, agent_step, agent_id),
            );
        }

        let result = handle_agent_send(&ctx, query.to_string(), "hello".to_string()).await;
        assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
    });
}

#[tokio::test]
async fn agent_send_finds_standalone_crew() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        s.crew.insert(
            "run-1".to_string(),
            oj_core::Crew::builder()
                .id("run-1")
                .agent_name("my-agent")
                .command_name("oj crew")
                .project("proj")
                .cwd("/tmp")
                .runbook_hash("hash")
                .agent_id("standalone-agent-42")
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
        let mut job = make_job("job-multi", "done");
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
        s.jobs.insert("job-multi".to_string(), job);
    }

    let result = handle_agent_send(&ctx, "job-multi".to_string(), "hello".to_string()).await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}
