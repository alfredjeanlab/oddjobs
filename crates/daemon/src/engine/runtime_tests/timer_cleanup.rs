// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer lifecycle cleanup tests.
//!
//! Verifies that liveness, exit-deferred, and cooldown timers are properly
//! cleaned up when jobs advance, complete, fail, or are cancelled.

use super::*;
use oj_core::{JobId, TimerId};

// =============================================================================
// Liveness timer cleanup
// =============================================================================

#[tokio::test]
async fn liveness_timer_cancelled_when_job_advances_past_agent_step() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    // Liveness timer should be pending from spawn
    let scheduler = ctx.runtime.executor.scheduler();
    assert!(scheduler.lock().has_timers());

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent goes idle → on_idle = done → job advances to "finish"
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "finish");

    // Liveness timer should be cancelled (not pending)
    let timer_ids = ctx.pending_timer_ids();
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", job_id));
}

/// Shared helper: verify a timer is cancelled when a job terminates via failure or cancellation.
///
/// If `setup_exit_deferred` is true, fires the liveness timer first to schedule an
/// exit-deferred timer (used for exit-deferred timer cleanup tests).
async fn assert_timer_cancelled_on_termination(
    timer_prefix: &str,
    terminate_via_cancel: bool,
    setup_exit_deferred: bool,
) {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    if setup_exit_deferred {
        // Mark agent as dead so liveness detects death → schedules exit-deferred
        let agent_id = get_agent_id(&ctx, &job_id).unwrap();
        ctx.agents.set_agent_alive(&agent_id, false);
        ctx.runtime
            .handle_event(Event::TimerStart {
                id: TimerId::liveness(&JobId::from_string(job_id.clone())),
            })
            .await
            .unwrap();
    }

    if terminate_via_cancel {
        ctx.runtime
            .handle_event(Event::JobCancel { id: JobId::from_string(job_id.clone()) })
            .await
            .unwrap();
    } else {
        ctx.runtime.handle_event(shell_fail(&job_id, "work")).await.unwrap();
    }

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.is_terminal());

    let timer_ids = ctx.pending_timer_ids();
    assert_no_timer_with_prefix(&timer_ids, &format!("{}:{}", timer_prefix, job_id));
}

#[tokio::test]
async fn liveness_timer_cancelled_on_job_failure() {
    assert_timer_cancelled_on_termination("liveness", false, false).await;
}

#[tokio::test]
async fn liveness_timer_cancelled_on_job_cancellation() {
    assert_timer_cancelled_on_termination("liveness", true, false).await;
}

#[tokio::test]
async fn exit_deferred_timer_cancelled_when_job_advances() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "work");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Mark agent dead → liveness detects death → schedules exit-deferred
    ctx.agents.set_agent_alive(&agent_id, false);
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::from_string(job_id.clone())),
        })
        .await
        .unwrap();

    // Verify exit-deferred was scheduled
    {
        let scheduler = ctx.runtime.executor.scheduler();
        assert!(scheduler.lock().has_timers());
    }

    // Agent goes idle → on_idle = done → job advances (before exit-deferred fires)
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "finish");

    // Exit-deferred timer should have been cancelled during advance
    let timer_ids = ctx.pending_timer_ids();
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", job_id));
}

#[tokio::test]
async fn exit_deferred_timer_cancelled_on_job_failure() {
    assert_timer_cancelled_on_termination("exit-deferred", false, true).await;
}

#[tokio::test]
async fn exit_deferred_timer_cancelled_on_job_cancellation() {
    assert_timer_cancelled_on_termination("exit-deferred", true, true).await;
}

#[tokio::test]
async fn cooldown_timer_noop_when_job_becomes_terminal() {
    let mut ctx = setup_with_runbook(&test_runbook("work", "finish", "run = \"claude --print\"\non_idle = { action = \"nudge\", attempts = 3, cooldown = \"10s\" }\non_dead = \"done\"")).await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // First idle → on_idle nudge (attempt 1, no cooldown yet)
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    // Second idle → attempt 2 → cooldown timer scheduled
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    // Verify cooldown timer was scheduled
    {
        let scheduler = ctx.runtime.executor.scheduler();
        let sched = scheduler.lock();
        assert!(sched.has_timers(), "cooldown timer should be pending");
    }

    // Fail the job while cooldown is pending
    ctx.runtime.handle_event(shell_fail(&job_id, "work")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.is_terminal());

    // Fire the cooldown timer — should be a no-op since job is terminal
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::cooldown(&JobId::from_string(job_id.clone()), "idle", 0),
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "cooldown on terminal job should be a no-op");
}

#[tokio::test]
async fn cooldown_timer_noop_when_job_missing() {
    let ctx = setup_with_runbook(&test_runbook("work", "finish", "run = \"claude --print\"\non_idle = { action = \"nudge\", attempts = 3, cooldown = \"10s\" }\non_dead = \"done\"")).await;

    // Fire cooldown timer for a job that doesn't exist
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::cooldown(&JobId::from_string("nonexistent"), "idle", 0),
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn all_job_timers_cancelled_after_on_dead_done_completes() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Full lifecycle: liveness detects dead agent → exit-deferred → on_dead=done
    // Step 1: Mark agent as dead, fire liveness → exit-deferred scheduled
    ctx.agents.set_agent_alive(&agent_id, false);
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::from_string(job_id.clone())),
        })
        .await
        .unwrap();

    // Step 2: Set agent state to Exited for on_dead
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::Exited { exit_code: Some(0) });

    // Step 3: Exit-deferred fires → on_dead=done → job advances to "finish"
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::from_string(job_id.clone())),
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "finish");

    // No liveness or exit-deferred timers should remain for this job
    let timer_ids = ctx.pending_timer_ids();
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", job_id));
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", job_id));
}

#[tokio::test]
async fn all_job_timers_cancelled_after_on_idle_done_completes() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "finish",
        "run = \"claude --print\"\non_idle = \"done\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent dead → liveness → exit-deferred (both timers now exist)
    ctx.agents.set_agent_alive(&agent_id, false);
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::from_string(job_id.clone())),
        })
        .await
        .unwrap();

    // Agent goes idle → on_idle=done → job advances to "finish"
    // This should cancel BOTH liveness and exit-deferred timers
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "finish");

    // Both timers should be gone
    let timer_ids = ctx.pending_timer_ids();
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", job_id));
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", job_id));
}
