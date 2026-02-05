// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Idle grace timer tests and nudge suppression

use super::*;

// =============================================================================
// Idle grace timer tests
// =============================================================================

/// AgentIdle sets a grace timer and records log size; doesn't immediately trigger on_idle.
#[tokio::test]
async fn idle_grace_timer_set_on_agent_idle() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set a known log size
    ctx.agents.set_session_log_size(&agent_id, Some(42));

    // AgentIdle should NOT immediately escalate
    let result = ctx
        .runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "AgentIdle should produce no immediate events (grace timer defers action)"
    );

    // Job should still be Running (not escalated)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);

    // Grace timer should be scheduled
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(std::time::Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        timer_ids.iter().any(|id| id.starts_with("idle-grace:")),
        "idle grace timer should be scheduled, found: {:?}",
        timer_ids
    );

    // Log size should be recorded on the job
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.idle_grace_log_size, Some(42));
}

/// Second AgentIdle while grace timer is pending is a no-op (deduplication).
#[tokio::test]
async fn idle_grace_timer_deduplicates() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // First AgentIdle -> sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.idle_grace_log_size, Some(100));

    // Increase log size to simulate activity
    ctx.agents.set_session_log_size(&agent_id, Some(200));

    // Second AgentIdle -> should be deduplicated (idle_grace_log_size already set)
    let result = ctx
        .runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "duplicate AgentIdle should be no-op");

    // Log size should NOT be updated (still 100 from first idle)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.idle_grace_log_size, Some(100));
}

/// Working state cancels pending idle grace timer.
#[tokio::test]
async fn idle_grace_timer_cancelled_on_working() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // AgentIdle -> sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.idle_grace_log_size.is_some());

    // AgentWorking -> should cancel grace timer and clear log size
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.idle_grace_log_size, None,
        "idle_grace_log_size should be cleared on Working"
    );
}

/// Grace timer fires but log grew -> no action (agent was active during grace period).
#[tokio::test]
async fn idle_grace_timer_noop_when_log_grew() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // AgentIdle -> sets grace timer, records log_size=100
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Simulate log growth during grace period
    ctx.agents.set_session_log_size(&agent_id, Some(200));
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Fire the grace timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "grace timer should produce no events when log grew"
    );

    // Job should still be Running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);
}

/// Grace timer fires, log unchanged but agent is Working -> no action (race guard).
#[tokio::test]
async fn idle_grace_timer_noop_when_agent_working() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // AgentIdle -> sets grace timer, records log_size=100
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Log hasn't grown, but agent started working (race condition guard)
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::Working);

    // Fire the grace timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "grace timer should produce no events when agent is Working"
    );

    // Job should still be Running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);
}

/// Grace timer fires, log unchanged + agent WaitingForInput -> proceeds with on_idle.
#[tokio::test]
async fn idle_grace_timer_proceeds_when_genuinely_idle() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    ctx.agents.set_session_log_size(&agent_id, Some(100));
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // AgentIdle -> sets grace timer, records log_size=100
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer — log unchanged, agent idle -> should proceed with on_idle
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // on_idle = escalate -> job should be Waiting
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(
        job.step_status.is_waiting(),
        "job should be Waiting after genuine idle triggers on_idle=escalate"
    );
}

/// Working state within 60s of nudge doesn't auto-resume or reset attempts.
#[tokio::test]
async fn auto_resume_suppressed_after_nudge() {
    let ctx = setup_with_runbook(RUNBOOK_GATE_IDLE_FAIL).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Put job into Waiting state via AgentWaiting (direct monitor path)
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());

    // Simulate a nudge having been sent recently by setting last_nudge_at
    let now = ctx.clock.epoch_ms();
    let pid = JobId::new(&job_id);
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(pid.as_str()) {
            p.last_nudge_at = Some(now);
        }
    });

    // Agent starts working (likely from our nudge text)
    let result = ctx
        .runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "auto-resume should be suppressed within 60s of nudge"
    );

    // Job should still be Waiting (not resumed to Running)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(
        job.step_status.is_waiting(),
        "job should remain Waiting when Working is suppressed after nudge"
    );
}

/// Working state after 60s of nudge allows normal auto-resume.
#[tokio::test]
async fn auto_resume_allowed_after_nudge_cooldown() {
    let ctx = setup_with_runbook(RUNBOOK_GATE_IDLE_FAIL).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Put job into Waiting state
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());

    // Set last_nudge_at to 61 seconds ago
    let now = ctx.clock.epoch_ms();
    let pid = JobId::new(&job_id);
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(pid.as_str()) {
            p.last_nudge_at = Some(now.saturating_sub(61_000));
        }
    });

    // Agent starts working after cooldown period
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Job should be auto-resumed to Running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.step_status,
        StepStatus::Running,
        "job should auto-resume after nudge cooldown expires"
    );
}

/// Rapid AgentIdle/Working cycling (simulating inter-tool-call gaps) never triggers nudge.
#[tokio::test]
async fn rapid_idle_working_cycling_no_nudge() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // Simulate 5 rapid idle/working cycles (like between tool calls)
    for i in 0..5 {
        ctx.agents
            .set_session_log_size(&agent_id, Some(100 + i * 50));

        // AgentIdle -> sets grace timer
        ctx.runtime
            .handle_event(Event::AgentIdle {
                agent_id: agent_id.clone(),
            })
            .await
            .unwrap();

        // AgentWorking -> cancels grace timer
        ctx.runtime
            .handle_event(Event::AgentWorking {
                agent_id: agent_id.clone(),
                owner: None,
            })
            .await
            .unwrap();
    }

    // Job should still be Running — no escalation happened
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(
        job.step_status,
        StepStatus::Running,
        "rapid idle/working cycling should never trigger on_idle"
    );
    assert_eq!(
        job.idle_grace_log_size, None,
        "grace log size should be cleared after Working"
    );
}
