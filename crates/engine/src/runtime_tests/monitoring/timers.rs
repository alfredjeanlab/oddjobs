// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Liveness timer and deferred exit timer tests

use super::*;

// =============================================================================
// Liveness timer happy paths
// =============================================================================

#[tokio::test]
async fn liveness_timer_reschedules_when_session_alive() {
    let ctx = setup().await;
    let (job_id, session_id, _agent_id) = setup_job_at_agent_step(&ctx).await;

    // Register the session as alive in the fake adapter
    ctx.sessions.add_session(&session_id, true);

    // Fire the liveness timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // Liveness check when alive produces no events (just reschedules the timer)
    assert!(result.is_empty());

    // Verify the liveness timer was rescheduled (not an exit-deferred timer)
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    assert!(sched.has_timers());
    ctx.clock.advance(std::time::Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(timer_ids.contains(&TimerId::liveness(&JobId::new(job_id.clone())).as_str()));
    assert!(!timer_ids.iter().any(|id| id.starts_with("exit-deferred:")));
}

#[tokio::test]
async fn liveness_timer_schedules_deferred_exit_when_session_dead() {
    let ctx = setup().await;
    let (job_id, _session_id, _agent_id) = setup_job_at_agent_step(&ctx).await;

    // Don't add the session to FakeSessionAdapter — is_alive returns false for unknown sessions

    // Fire the liveness timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // Dead session produces no direct events (schedules deferred exit timer)
    assert!(result.is_empty());

    // Verify a deferred exit timer was scheduled
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    assert!(sched.has_timers());
    ctx.clock.advance(std::time::Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(timer_ids.contains(&TimerId::exit_deferred(&JobId::new(job_id.clone())).as_str()));
}

// =============================================================================
// Deferred exit timer happy paths
// =============================================================================

#[tokio::test]
async fn exit_deferred_timer_noop_when_job_terminal() {
    let ctx = setup().await;
    let (job_id, _session_id, _agent_id) = setup_job_at_agent_step(&ctx).await;

    // Fail the job to make it terminal
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "plan".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.is_terminal());

    // Deferred exit on a terminal job should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn exit_deferred_timer_on_idle_when_waiting_for_input() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let job_id = create_job(&ctx).await;

    // Advance to agent step
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent state to WaitingForInput
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // With on_idle = done, job should advance past the agent step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

#[tokio::test]
async fn exit_deferred_timer_on_error_when_agent_failed() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let job_id = create_job(&ctx).await;

    // Advance to agent step
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent state to Failed
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Failed(oj_core::AgentError::Other("test error".to_string())),
    );

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // With on_error = fail, job should be failed
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed");
}

#[tokio::test]
async fn exit_deferred_timer_on_dead_for_exited_state() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let job_id = create_job(&ctx).await;

    // Advance to agent step
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent state to Exited (maps to on_dead fallback)
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Exited { exit_code: Some(0) },
    );

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // With on_dead = done, job should advance
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}
