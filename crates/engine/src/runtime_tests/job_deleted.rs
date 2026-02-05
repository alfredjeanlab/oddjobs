// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for cascading cleanup when jobs are deleted.
//!
//! Verifies that JobDeleted events trigger proper cleanup of:
//! - Timers (liveness, exit-deferred, cooldown)
//! - Agent→job mappings
//! - Sessions
//! - Workspaces

use super::*;
use oj_adapters::SessionCall;
use oj_core::{Event, JobId};

/// Collect all pending timer IDs from the scheduler.
fn pending_timer_ids(ctx: &TestContext) -> Vec<String> {
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(std::time::Duration::from_secs(7200));
    let fired = sched.fired_timers(ctx.clock.now());
    fired
        .into_iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str().to_string()),
            _ => None,
        })
        .collect()
}

/// Helper: check that no job-scoped timer with the given prefix exists.
fn assert_no_timer_with_prefix(timer_ids: &[String], prefix: &str) {
    let matching: Vec<&String> = timer_ids
        .iter()
        .filter(|id| id.starts_with(prefix))
        .collect();
    assert!(
        matching.is_empty(),
        "expected no timers starting with '{}', found: {:?}",
        prefix,
        matching
    );
}

// =============================================================================
// Runbook definitions
// =============================================================================

/// Runbook with an agent step that triggers timers
const RUNBOOK_WITH_AGENT: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]

[[job.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "finish"

[[job.build.step]]
name = "finish"
run = "echo done"

[agent.worker]
run = "claude --print"
on_idle = "done"
on_dead = "done"
"#;

// =============================================================================
// Timer cancellation tests
// =============================================================================

#[tokio::test]
async fn job_deleted_cancels_timers() {
    let ctx = setup_with_runbook(RUNBOOK_WITH_AGENT).await;

    // Create a job that will be on an agent step (creates timers)
    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = "pipe-1".to_string();
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    // Liveness timer should be pending from spawn
    let scheduler = ctx.runtime.executor.scheduler();
    assert!(
        scheduler.lock().has_timers(),
        "should have timers before delete"
    );

    // Now delete the job
    ctx.runtime
        .handle_event(Event::JobDeleted {
            id: JobId::new(&job_id),
        })
        .await
        .unwrap();

    // All job-scoped timers should be cancelled
    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", job_id));
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", job_id));
    assert_no_timer_with_prefix(&timer_ids, &format!("cooldown:{}", job_id));
}

// =============================================================================
// Agent mapping tests
// =============================================================================

#[tokio::test]
async fn job_deleted_deregisters_agent_mapping() {
    let ctx = setup_with_runbook(RUNBOOK_WITH_AGENT).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = "pipe-1".to_string();

    // Get the agent_id that was assigned
    let agent_id = get_agent_id(&ctx, &job_id).expect("should have agent_id in step history");

    // Verify agent→job mapping exists
    {
        let agent_jobs = ctx.runtime.agent_jobs.lock();
        assert!(
            agent_jobs.contains_key(&agent_id),
            "agent mapping should exist before delete"
        );
    }

    // Delete the job
    ctx.runtime
        .handle_event(Event::JobDeleted {
            id: JobId::new(&job_id),
        })
        .await
        .unwrap();

    // Agent→job mapping should be removed
    {
        let agent_jobs = ctx.runtime.agent_jobs.lock();
        assert!(
            !agent_jobs.contains_key(&agent_id),
            "agent mapping should be removed after delete"
        );
    }
}

// =============================================================================
// Session cleanup tests
// =============================================================================

#[tokio::test]
async fn job_deleted_kills_session() {
    let ctx = setup_with_runbook(RUNBOOK_WITH_AGENT).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = "pipe-1".to_string();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    // Get the session_id
    let session_id = job.session_id.clone().expect("job should have session_id");

    // Verify session exists
    assert!(
        ctx.runtime
            .lock_state(|s| s.sessions.contains_key(&session_id)),
        "session should exist before delete"
    );

    // Delete the job
    ctx.runtime
        .handle_event(Event::JobDeleted {
            id: JobId::new(&job_id),
        })
        .await
        .unwrap();

    // Check that KillSession was called via the fake adapter
    let calls = ctx.sessions.calls();
    let kill_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, SessionCall::Kill { id } if id == &session_id))
        .collect();
    assert!(
        !kill_calls.is_empty(),
        "session should have been killed; calls: {:?}",
        calls
    );
}

// =============================================================================
// Idempotency tests
// =============================================================================

#[tokio::test]
async fn job_deleted_idempotent_for_missing_job() {
    let ctx = setup_with_runbook(RUNBOOK_WITH_AGENT).await;

    // Delete a job that doesn't exist
    let result = ctx
        .runtime
        .handle_event(Event::JobDeleted {
            id: JobId::new("nonexistent-job"),
        })
        .await;

    // Should not error
    assert!(result.is_ok(), "deleting nonexistent job should not error");
}

#[tokio::test]
async fn job_deleted_idempotent_when_resources_already_gone() {
    let ctx = setup_with_runbook(RUNBOOK_WITH_AGENT).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = "pipe-1".to_string();

    // First delete
    ctx.runtime
        .handle_event(Event::JobDeleted {
            id: JobId::new(&job_id),
        })
        .await
        .unwrap();

    // Second delete (resources already cleaned up)
    let result = ctx
        .runtime
        .handle_event(Event::JobDeleted {
            id: JobId::new(&job_id),
        })
        .await;

    // Should not error
    assert!(
        result.is_ok(),
        "duplicate job delete should not error (idempotent)"
    );
}

#[tokio::test]
async fn job_deleted_handles_terminal_job() {
    let ctx = setup_with_runbook(RUNBOOK_WITH_AGENT).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = "pipe-1".to_string();

    // Advance to completion (terminal state)
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Job should be on "finish" step now
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "finish");

    // Complete the shell step
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(&job_id),
            step: "finish".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Job should be terminal (done)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.is_terminal(), "job should be terminal");

    // Delete the terminal job
    let result = ctx
        .runtime
        .handle_event(Event::JobDeleted {
            id: JobId::new(&job_id),
        })
        .await;

    assert!(result.is_ok(), "deleting terminal job should succeed");
}
