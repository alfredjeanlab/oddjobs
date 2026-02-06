// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for attempt tracking and exhaustion with cooldowns.

use super::*;

#[tokio::test]
async fn standalone_on_idle_exhausts_attempts_then_escalates() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_IDLE_ATTEMPTS).await;

    ctx.runtime
        .handle_event(command_event(
            "run-1",
            "build",
            "agent_cmd",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let (agent_run_id, _session_id, agent_id) = {
        let ar = ctx
            .runtime
            .lock_state(|s| s.agent_runs.get("run-1").cloned())
            .unwrap();
        (
            ar.id.clone(),
            ar.session_id.clone().unwrap(),
            AgentId::new(ar.agent_id.as_ref().unwrap()),
        )
    };

    // Register session so nudge doesn't fail
    ctx.sessions.add_session(
        &ctx.runtime
            .lock_state(|s| s.agent_runs.get(&agent_run_id).unwrap().session_id.clone())
            .unwrap(),
        true,
    );

    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle → attempt 1 (nudge)
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: OwnerId::AgentRun(AgentRunId::new("run-1")),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Running, "first nudge");

    // Second idle → attempt 2 (nudge)
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: OwnerId::AgentRun(AgentRunId::new("run-1")),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Running, "second nudge");

    // Third idle → attempts exhausted (2), should escalate
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: OwnerId::AgentRun(AgentRunId::new("run-1")),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(
        agent_run.status,
        AgentRunStatus::Escalated,
        "should escalate after exhausting attempts"
    );
}

#[tokio::test]
async fn standalone_on_idle_cooldown_schedules_timer() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_IDLE_COOLDOWN).await;

    ctx.runtime
        .handle_event(command_event(
            "run-1",
            "build",
            "agent_cmd",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let (_agent_run_id, session_id, agent_id) = {
        let ar = ctx
            .runtime
            .lock_state(|s| s.agent_runs.get("run-1").cloned())
            .unwrap();
        (
            ar.id.clone(),
            ar.session_id.clone().unwrap(),
            AgentId::new(ar.agent_id.as_ref().unwrap()),
        )
    };

    ctx.sessions.add_session(&session_id, true);
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle → attempt 1 (immediate, no cooldown)
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: OwnerId::AgentRun(AgentRunId::new("run-1")),
        })
        .await
        .unwrap();

    // Second idle → attempt 2, but cooldown required
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: OwnerId::AgentRun(AgentRunId::new("run-1")),
        })
        .await
        .unwrap();

    // Cooldown timer should be scheduled
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(std::time::Duration::from_secs(60));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        timer_ids.iter().any(|id| id.starts_with("cooldown:ar:")),
        "cooldown timer should be scheduled, found: {:?}",
        timer_ids
    );
}
