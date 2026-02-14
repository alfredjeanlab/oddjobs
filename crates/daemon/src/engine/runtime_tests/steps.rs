// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Step transition tests

use super::*;
use oj_core::{JobId, TimerId};
use std::time::Duration;

#[tokio::test]
async fn shell_failure_fails_job() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Simulate shell failure
    ctx.runtime.handle_event(shell_fail(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed");
}

#[tokio::test]
async fn agent_error_fails_job() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to plan step
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    // Simulate agent failure via fail_job (orchestrator-driven)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.fail_job(&job, "timeout").await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed");
}

#[tokio::test]
async fn on_fail_transition_executes() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to merge step (which has on_fail = "cleanup")
    // init -> plan -> execute -> merge
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    // Advance through agent steps (plan -> execute -> merge)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "merge");

    // Simulate merge failure - should transition to cleanup (custom step)
    ctx.runtime.handle_event(shell_fail(&job_id, "merge")).await.unwrap();

    // With string-based steps, custom steps like "cleanup" now work correctly
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "cleanup", "Expected cleanup step, got {}", job.step);
}

#[tokio::test]
async fn final_step_completes_job() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance through all steps to done
    // init -> plan -> execute -> merge -> done
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    // Advance through agent steps (plan -> execute -> merge)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    ctx.runtime.handle_event(shell_ok(&job_id, "merge")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

#[tokio::test]
async fn done_step_run_command_executes() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance through all steps: init -> plan -> execute -> merge -> done
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    // Advance through agent steps (plan -> execute -> merge)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    ctx.runtime.handle_event(shell_ok(&job_id, "merge")).await.unwrap();

    // At this point, job should be in Done step with Running status
    // (because the "done" step's run command is executing)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Running);

    // Complete the "done" step's shell command
    ctx.runtime.handle_event(shell_ok(&job_id, "done")).await.unwrap();

    // Now job should be Done with Completed status
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Completed);
}

#[tokio::test]
async fn wrong_step_shell_completed_ignored() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Try to complete a step we're not in
    ctx.runtime
        .handle_event(shell_ok(&job_id, "merge")) // We're in init, not merge
        .await
        .unwrap();

    // Should still be in init
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "init");
}

/// Runbook without explicit on_done - step should complete the job
#[tokio::test]
async fn step_without_on_done_completes_job() {
    let ctx = setup_with_runbook(&test_runbook_shell("simple", "")).await;

    // Create job
    let job_id = create_job_for_runbook(&ctx, "simple", &[]).await;

    // Complete init - no on_done means job should complete, not advance sequentially
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Completed);
}

#[tokio::test]
async fn explicit_next_step_is_followed() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "build",
        "",
        &[
            ("init", "echo init", "on_done = { step = \"custom\" }"),
            ("custom", "echo custom", "on_done = { step = \"done\" }"),
            ("done", "echo done", ""),
        ],
    ))
    .await;

    // Create job
    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    // Complete init - should go to custom (not second step in order)
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "custom");

    // Complete custom - should go to done (from explicit next)
    ctx.runtime.handle_event(shell_ok(&job_id, "custom")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

#[tokio::test]
async fn implicit_done_step_completes_immediately() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "build",
        "",
        &[("init", "echo init", "on_done = { step = \"done\" }"), ("done", "true", "")],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    // Complete init - should advance to done step
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Running);

    // Complete done step - job should complete
    ctx.runtime.handle_event(shell_ok(&job_id, "done")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Completed);
}

#[tokio::test]
async fn step_runs_with_fallback_workspace_path() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "init");

    // Shell completion should work even if workspace_path might not be set
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");
}

#[tokio::test]
async fn advance_job_cancels_exit_deferred_timer() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to plan step (agent)
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");

    // Manually schedule an exit-deferred timer (simulates liveness detecting death)
    {
        let scheduler = ctx.runtime.executor.scheduler();
        let mut sched = scheduler.lock();
        sched.set_timer(
            TimerId::exit_deferred(&JobId::from_string(job_id.clone())).to_string(),
            Duration::from_secs(5),
            ctx.clock.now(),
        );
    }

    // Advance job past the agent step
    ctx.runtime.advance_job(&job).await.unwrap();

    // Verify exit-deferred timer is cancelled
    // (liveness timer may be re-created if the next step is also an agent)
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        !timer_ids.contains(&TimerId::exit_deferred(&JobId::from_string(job_id.clone())).as_str()),
        "advance_job must cancel exit-deferred timer"
    );
}

#[tokio::test]
async fn fail_job_cancels_exit_deferred_timer() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to plan step (agent)
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");

    // Manually schedule an exit-deferred timer (simulates liveness detecting death)
    {
        let scheduler = ctx.runtime.executor.scheduler();
        let mut sched = scheduler.lock();
        sched.set_timer(
            TimerId::exit_deferred(&JobId::from_string(job_id.clone())).to_string(),
            Duration::from_secs(5),
            ctx.clock.now(),
        );
    }

    // Fail the job from the agent step
    ctx.runtime.fail_job(&job, "test failure").await.unwrap();

    // Verify both timers are cancelled
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        !timer_ids.contains(&TimerId::exit_deferred(&JobId::from_string(job_id.clone())).as_str()),
        "fail_job must cancel exit-deferred timer"
    );
    assert!(
        !timer_ids.contains(&TimerId::liveness(&JobId::from_string(job_id.clone())).as_str()),
        "fail_job must cancel liveness timer"
    );
}
