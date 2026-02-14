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
use crate::adapters::AgentCall;

// =============================================================================
// Timer cancellation tests
// =============================================================================

#[tokio::test]
async fn job_deleted_cancels_timers() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    // Create a job that will be on an agent step (creates timers)
    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    // Liveness timer should be pending from spawn
    let scheduler = ctx.runtime.executor.scheduler();
    assert!(scheduler.lock().has_timers(), "should have timers before delete");

    // Now delete the job
    ctx.runtime.handle_event(Event::JobDeleted { id: JobId::from_string(&job_id) }).await.unwrap();

    // All job-scoped timers should be cancelled
    let timer_ids = ctx.pending_timer_ids();
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", job_id));
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", job_id));
    assert_no_timer_with_prefix(&timer_ids, &format!("cooldown:{}", job_id));
}

#[tokio::test]
async fn job_deleted_deregisters_agent_mapping() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    // Get the agent_id that was assigned
    let agent_id = get_agent_id(&ctx, &job_id).expect("should have agent_id in step history");

    // Verify agent→job mapping exists
    {
        let agent_owners = ctx.runtime.agent_owners.lock();
        assert!(agent_owners.contains_key(&agent_id), "agent mapping should exist before delete");
    }

    // Delete the job
    ctx.runtime.handle_event(Event::JobDeleted { id: JobId::from_string(&job_id) }).await.unwrap();

    // Agent→job mapping should be removed
    {
        let agent_owners = ctx.runtime.agent_owners.lock();
        assert!(
            !agent_owners.contains_key(&agent_id),
            "agent mapping should be removed after delete"
        );
    }
}

#[tokio::test]
async fn job_deleted_kills_agent() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    let agent_id = get_agent_id(&ctx, &job_id).expect("should have agent_id in step history");

    // Delete the job
    ctx.runtime.handle_event(Event::JobDeleted { id: JobId::from_string(&job_id) }).await.unwrap();

    // Yield to let fire-and-forget KillAgent task complete
    tokio::task::yield_now().await;

    // Check that KillAgent was called via the fake adapter
    let calls = ctx.agents.calls();
    let kill_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, AgentCall::Kill { agent_id: aid } if *aid == agent_id))
        .collect();
    assert!(!kill_calls.is_empty(), "agent should have been killed; calls: {:?}", calls);
}

#[tokio::test]
async fn job_deleted_idempotent_for_missing_job() {
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    // Delete a job that doesn't exist
    let result = ctx
        .runtime
        .handle_event(Event::JobDeleted { id: JobId::from_string("nonexistent-job") })
        .await;

    // Should not error
    assert!(result.is_ok(), "deleting nonexistent job should not error");
}

#[tokio::test]
async fn job_deleted_idempotent_when_resources_already_gone() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    // First delete
    ctx.runtime.handle_event(Event::JobDeleted { id: JobId::from_string(&job_id) }).await.unwrap();

    // Second delete (resources already cleaned up)
    let result =
        ctx.runtime.handle_event(Event::JobDeleted { id: JobId::from_string(&job_id) }).await;

    // Should not error
    assert!(result.is_ok(), "duplicate job delete should not error (idempotent)");
}

#[tokio::test]
async fn job_deleted_handles_terminal_job() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    // Advance to completion (terminal state)
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    // Job should be on "finish" step now
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "finish");

    // Complete the shell step
    ctx.runtime.handle_event(shell_ok(&job_id, "finish")).await.unwrap();

    // Job should be terminal (done)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.is_terminal(), "job should be terminal");

    // Delete the terminal job
    let result =
        ctx.runtime.handle_event(Event::JobDeleted { id: JobId::from_string(&job_id) }).await;

    assert!(result.is_ok(), "deleting terminal job should succeed");
}
