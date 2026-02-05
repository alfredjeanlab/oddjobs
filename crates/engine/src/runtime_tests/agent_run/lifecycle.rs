// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for agent registration, liveness timers, and working state transitions.

use super::*;

// =============================================================================
// Register/deregister agent run mapping tests
// =============================================================================

#[tokio::test]
async fn register_agent_adds_mapping() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_RECOVERY).await;

    let agent_id = AgentId::new("test-agent");
    let agent_run_id = AgentRunId::new("test-run");

    // Register mapping
    ctx.runtime
        .register_agent(agent_id.clone(), OwnerId::agent_run(agent_run_id.clone()));

    // Verify mapping exists
    let mapped_owner = ctx.runtime.agent_owners.lock().get(&agent_id).cloned();
    assert_eq!(mapped_owner, Some(OwnerId::agent_run(agent_run_id)));
}

// =============================================================================
// Liveness timer tests for standalone agents
// =============================================================================

#[tokio::test]
async fn standalone_liveness_timer_reschedules_when_alive() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_RECOVERY).await;

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

    let (agent_run_id, session_id, _agent_id) = {
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

    // Register session as alive
    ctx.sessions.add_session(&session_id, true);

    // Fire liveness timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness_agent_run(&AgentRunId::new(&agent_run_id)),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "liveness check when alive produces no events"
    );

    // Verify liveness timer was rescheduled
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

    assert!(timer_ids.iter().any(|id| id.starts_with("liveness:ar:")));
}

// =============================================================================
// Working state clears idle grace timer
// =============================================================================

#[tokio::test]
async fn standalone_working_clears_idle_grace() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_RECOVERY).await;

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

    let (agent_run_id, session_id, agent_id) = {
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
    ctx.agents.set_session_log_size(&agent_id, Some(100));
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // AgentIdle sets grace timer and records log size
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert!(agent_run.idle_grace_log_size.is_some());

    // AgentWorking clears grace state
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(
        agent_run.idle_grace_log_size, None,
        "Working should clear idle_grace_log_size"
    );
}
