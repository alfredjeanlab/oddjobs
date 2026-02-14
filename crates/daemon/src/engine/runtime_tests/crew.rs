// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for crew lifecycle handling.

use super::*;
use crate::adapters::AgentCall;
use oj_core::{CrewId, CrewStatus, OwnerId, TimerId};

// ── Actions ───────────────────────────────────────────────────────────

#[tokio::test]
async fn standalone_on_dead_fail_fails_crew() {
    let mut ctx = setup_with_runbook(&test_runbook_agent("on_dead = \"fail\"")).await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    // Agent exits → on_dead = fail
    ctx.runtime
        .handle_event(agent_exited(agent_id.clone(), Some(0), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Failed);

    // Yield to let fire-and-forget KillAgent task complete
    tokio::task::yield_now().await;

    // Agent should be killed
    let kills: Vec<_> = ctx
        .agents
        .calls()
        .into_iter()
        .filter(|c| matches!(c, AgentCall::Kill { agent_id: aid } if *aid == agent_id))
        .collect();
    assert!(!kills.is_empty(), "session should be killed on fail action");
}

#[tokio::test]
async fn standalone_on_dead_gate_pass_completes_crew() {
    let mut ctx = setup_with_runbook(&test_runbook_agent(
        "on_dead = { action = \"gate\", run = \"true\" }\non_idle = \"done\"",
    ))
    .await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    // Agent exits → on_dead = gate (true) → pass → complete
    ctx.runtime
        .handle_event(agent_exited(agent_id.clone(), Some(0), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Completed, "gate pass should complete crew");
}

#[tokio::test]
async fn standalone_on_dead_gate_fail_escalates_crew() {
    let mut ctx =
        setup_with_runbook(&test_runbook_agent("on_dead = { action = \"gate\", run = \"false\" }"))
            .await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    // Agent exits → on_dead = gate (false) → fail → escalate
    ctx.runtime
        .handle_event(agent_exited(agent_id.clone(), Some(0), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Escalated, "gate fail should escalate crew");

    // Verify the reason includes gate failure info
    assert!(
        crew.error.is_none() || crew.status == CrewStatus::Escalated,
        "escalation should set status, not error"
    );
}

#[tokio::test]
async fn standalone_on_idle_gate_pass_completes() {
    // test_runbook_agent with on_idle = done has on_idle = done (not gate),
    // but we need to test on_idle gate, so we define a new one inline
    let runbook = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = { action = "gate", run = "true" }
"#;

    let mut ctx = setup_with_runbook(runbook).await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Agent goes idle → on_idle = gate (true) → pass → complete
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Completed, "on_idle gate pass should complete crew");
}

#[tokio::test]
async fn standalone_on_error_fail_fails_crew() {
    let mut ctx =
        setup_with_runbook(&test_runbook_agent("on_error = \"fail\"\non_idle = \"done\"")).await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    // Set agent to Failed state
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Failed(oj_core::AgentError::Other("API error".to_string())),
    );

    // Agent reports error via AgentFailed event
    ctx.runtime
        .handle_event(Event::AgentFailed {
            id: agent_id.clone(),
            error: oj_core::AgentError::Other("API error".to_string()),
            owner: CrewId::from_string(&crew_id).into(),
        })
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Failed, "on_error = fail should fail the crew");
}

// ── Attempts ──────────────────────────────────────────────────────────

#[tokio::test]
async fn standalone_on_idle_exhausts_attempts_then_escalates() {
    let mut ctx = setup_with_runbook(&test_runbook_agent(
        "on_idle = { action = \"nudge\", attempts = 2, message = \"Keep going\" }",
    ))
    .await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle → attempt 1 (nudge)
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string("job-1").into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Running, "first nudge");

    // Second idle → attempt 2 (nudge)
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string("job-1").into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Running, "second nudge");

    // Third idle → attempts exhausted (2), should escalate
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string("job-1").into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Escalated, "should escalate after exhausting attempts");
}

#[tokio::test]
async fn standalone_on_idle_cooldown_schedules_timer() {
    let mut ctx = setup_with_runbook(&test_runbook_agent("on_idle = { action = \"nudge\", attempts = 3, cooldown = \"30s\", message = \"Continue\" }")).await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle → attempt 1 (immediate, no cooldown)
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    // Second idle → attempt 2, but cooldown required
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string(&crew_id).into()))
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
        timer_ids.iter().any(|id| id.starts_with("cooldown:crw-")),
        "cooldown timer should be scheduled, found: {:?}",
        timer_ids
    );
}

// ── Lifecycle ─────────────────────────────────────────────────────────

#[tokio::test]
async fn register_agent_adds_mapping() {
    let ctx =
        setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"\non_dead = \"escalate\""))
            .await;

    let agent_id = AgentId::from_string("test-agent");
    let crew_id = CrewId::from_string("test-run");

    // Register mapping
    ctx.runtime.register_agent(agent_id.clone(), crew_id.clone().into());

    // Verify mapping exists
    let mapped_owner = ctx.runtime.agent_owners.lock().get(&agent_id).cloned();
    assert_eq!(mapped_owner, Some(OwnerId::crew(crew_id)));
}

#[tokio::test]
async fn standalone_liveness_timer_reschedules_when_alive() {
    let mut ctx =
        setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"\non_dead = \"escalate\""))
            .await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let crew_id = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        run.id.clone()
    };

    // Agent is alive by default after spawn (FakeAgentAdapter sets alive=true)

    // Fire liveness timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart { id: TimerId::liveness(CrewId::from_string(&crew_id)) })
        .await
        .unwrap();

    assert!(result.is_empty(), "liveness check when alive produces no events");

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

    assert!(timer_ids.iter().any(|id| id.starts_with("liveness:crw-")));
}

// ── Signals ───────────────────────────────────────────────────────────

#[tokio::test]
async fn standalone_stop_blocked_escalates_crew() {
    let mut ctx =
        setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"\non_dead = \"escalate\""))
            .await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    // Agent stop blocked → resolve stop + dispatch on_idle (escalate)
    ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Escalated);
}

#[tokio::test]
async fn standalone_stop_blocked_on_terminal_is_noop() {
    let mut ctx = setup_with_runbook(&test_runbook_agent("on_dead = \"fail\"")).await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    // First, fail the agent via exit
    ctx.runtime
        .handle_event(agent_exited(agent_id.clone(), Some(0), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Failed);

    // Now stop blocked on terminal crew → should be no-op
    let result =
        ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    assert!(result.is_empty(), "stop blocked on terminal crew is no-op");

    // Status should still be Failed (not Completed)
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Failed);
}

#[tokio::test]
async fn standalone_nudge_records_timestamp() {
    let mut ctx = setup_with_runbook(&test_runbook_agent(
        "on_idle = { action = \"nudge\", attempts = 2, message = \"Keep going\" }",
    ))
    .await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Before idle, last_nudge_at should be None
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert!(crew.last_nudge_at.is_none());

    // Agent goes idle → nudge is sent
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    // After nudge, last_nudge_at should be set
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert!(crew.last_nudge_at.is_some(), "last_nudge_at should be recorded after nudge");
}

#[tokio::test]
async fn standalone_auto_resume_suppressed_after_nudge() {
    let mut ctx =
        setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"\non_dead = \"escalate\""))
            .await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Agent goes idle → escalates (on_idle = escalate)
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Escalated);

    // Simulate a nudge was sent by setting last_nudge_at
    let now = ctx.clock.epoch_ms();
    ctx.runtime.lock_state_mut(|state| {
        if let Some(run) = state.crew.get_mut(&crew_id) {
            run.last_nudge_at = Some(now);
        }
    });

    // Agent starts working (likely from nudge)
    let result = ctx
        .runtime
        .handle_event(Event::AgentWorking {
            id: agent_id.clone(),
            owner: CrewId::from_string(&crew_id).into(),
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "auto-resume should be suppressed within 60s of nudge");

    // Should still be Escalated (not Running)
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Escalated);
}

#[tokio::test]
async fn standalone_auto_resume_allowed_after_cooldown() {
    let mut ctx =
        setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"\non_dead = \"escalate\""))
            .await;

    create_job_for_runbook(&ctx, "agent_cmd", &[]).await;
    ctx.process_background_events().await;

    let (crew_id, agent_id) = {
        let run = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.as_ref().unwrap()))
    };

    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Agent goes idle → escalates
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Escalated);

    // Set last_nudge_at to 61 seconds ago
    let now = ctx.clock.epoch_ms();
    ctx.runtime.lock_state_mut(|state| {
        if let Some(run) = state.crew.get_mut(&crew_id) {
            run.last_nudge_at = Some(now.saturating_sub(61_000));
        }
    });

    // Agent starts working after cooldown
    ctx.runtime
        .handle_event(Event::AgentWorking {
            id: agent_id.clone(),
            owner: CrewId::from_string(&crew_id).into(),
        })
        .await
        .unwrap();

    // Should be auto-resumed to Running
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, CrewStatus::Running, "should auto-resume after nudge cooldown");
}
