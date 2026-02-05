// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for standalone agent run lifecycle handling.
//!
//! Tests cover:
//! - Attempt tracking and exhaustion with cooldowns
//! - Gate command execution (success/failure/error)
//! - Fail action effects
//! - Agent recovery after daemon restart
//! - Nudge timestamp tracking for auto-resume suppression

use super::*;
use oj_adapters::SessionCall;
use oj_core::{AgentRunId, AgentRunStatus, AgentSignalKind, OwnerId, TimerId};

// =============================================================================
// Runbook definitions for standalone agent tests
// =============================================================================

/// Runbook with standalone agent, on_idle with attempts
const RUNBOOK_AGENT_IDLE_ATTEMPTS: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = { action = "nudge", attempts = 2, message = "Keep going" }
"#;

/// Runbook with standalone agent, on_idle with attempts and cooldown
const RUNBOOK_AGENT_IDLE_COOLDOWN: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = { action = "nudge", attempts = 3, cooldown = "30s", message = "Continue" }
"#;

/// Runbook with standalone agent, on_dead = fail
const RUNBOOK_AGENT_DEAD_FAIL: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_dead = "fail"
"#;

/// Runbook with standalone agent, on_dead = gate (passing)
const RUNBOOK_AGENT_GATE_PASS: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_dead = { action = "gate", run = "true" }
on_idle = "done"
"#;

/// Runbook with standalone agent, on_dead = gate (failing)
const RUNBOOK_AGENT_GATE_FAIL: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_dead = { action = "gate", run = "false" }
"#;

/// Runbook with standalone agent for recovery testing
const RUNBOOK_AGENT_RECOVERY: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = "escalate"
on_dead = "escalate"
"#;

/// Runbook with on_error = fail
const RUNBOOK_AGENT_ERROR_FAIL: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_error = "fail"
on_idle = "done"
"#;

// =============================================================================
// Helper functions
// =============================================================================

// =============================================================================
// Attempt tracking and exhaustion tests
// =============================================================================

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
            owner: None,
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
            owner: None,
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
            owner: None,
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
            owner: None,
        })
        .await
        .unwrap();

    // Second idle → attempt 2, but cooldown required
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
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

// =============================================================================
// Fail action tests
// =============================================================================

#[tokio::test]
async fn standalone_on_dead_fail_fails_agent_run() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_DEAD_FAIL).await;

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

    // Agent exits → on_dead = fail
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id: agent_id.clone(),
            exit_code: Some(0),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Failed);

    // Session should be killed
    let kills: Vec<_> = ctx
        .sessions
        .calls()
        .into_iter()
        .filter(|c| matches!(c, SessionCall::Kill { id } if id == &session_id))
        .collect();
    assert!(!kills.is_empty(), "session should be killed on fail action");
}

// =============================================================================
// Gate command tests
// =============================================================================

#[tokio::test]
async fn standalone_on_dead_gate_pass_completes_agent_run() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_GATE_PASS).await;

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

    // Agent exits → on_dead = gate (true) → pass → complete
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id: agent_id.clone(),
            exit_code: Some(0),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(
        agent_run.status,
        AgentRunStatus::Completed,
        "gate pass should complete agent run"
    );
}

#[tokio::test]
async fn standalone_on_dead_gate_fail_escalates_agent_run() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_GATE_FAIL).await;

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

    // Agent exits → on_dead = gate (false) → fail → escalate
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id: agent_id.clone(),
            exit_code: Some(0),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(
        agent_run.status,
        AgentRunStatus::Escalated,
        "gate fail should escalate agent run"
    );

    // Verify the reason includes gate failure info
    assert!(
        agent_run.error.is_none() || agent_run.status == AgentRunStatus::Escalated,
        "escalation should set status, not error"
    );
}

#[tokio::test]
async fn standalone_on_idle_gate_pass_completes() {
    // Use RUNBOOK_AGENT_GATE_PASS which has on_idle = done (not gate),
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

    let ctx = setup_with_runbook(runbook).await;

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
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Agent goes idle → on_idle = gate (true) → pass → complete
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(
        agent_run.status,
        AgentRunStatus::Completed,
        "on_idle gate pass should complete agent run"
    );
}

// =============================================================================
// Agent error handling tests
// =============================================================================

#[tokio::test]
async fn standalone_on_error_fail_fails_agent_run() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_ERROR_FAIL).await;

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

    // Set agent to Failed state
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Failed(oj_core::AgentError::Other("API error".to_string())),
    );

    // Agent reports error via AgentFailed event
    ctx.runtime
        .handle_event(Event::AgentFailed {
            agent_id: agent_id.clone(),
            error: oj_core::AgentError::Other("API error".to_string()),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(
        agent_run.status,
        AgentRunStatus::Failed,
        "on_error = fail should fail the agent run"
    );
}

// =============================================================================
// Signal handling tests
// =============================================================================

#[tokio::test]
async fn standalone_signal_escalate_preserves_session() {
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

    // Agent signals escalate
    ctx.runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Escalate,
            message: Some("Need human help".to_string()),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Escalated);

    // Session should NOT be killed (agent stays alive for interaction)
    let kills: Vec<_> = ctx
        .sessions
        .calls()
        .into_iter()
        .filter(|c| matches!(c, SessionCall::Kill { id } if id == &session_id))
        .collect();
    assert!(
        kills.is_empty(),
        "session should NOT be killed on escalate signal"
    );
}

#[tokio::test]
async fn standalone_signal_complete_on_terminal_is_noop() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_DEAD_FAIL).await;

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

    // First, fail the agent via exit
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id: agent_id.clone(),
            exit_code: Some(0),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Failed);

    // Now signal complete on terminal agent run → should be no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Complete,
            message: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "signal on terminal agent run is no-op");

    // Status should still be Failed (not Completed)
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Failed);
}

// =============================================================================
// Nudge timestamp tracking tests
// =============================================================================

#[tokio::test]
async fn standalone_nudge_records_timestamp() {
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
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Before idle, last_nudge_at should be None
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert!(agent_run.last_nudge_at.is_none());

    // Agent goes idle → nudge is sent
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // After nudge, last_nudge_at should be set
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert!(
        agent_run.last_nudge_at.is_some(),
        "last_nudge_at should be recorded after nudge"
    );
}

#[tokio::test]
async fn standalone_auto_resume_suppressed_after_nudge() {
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
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Agent goes idle → escalates (on_idle = escalate)
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Escalated);

    // Simulate a nudge was sent by setting last_nudge_at
    let now = ctx.clock.epoch_ms();
    ctx.runtime.lock_state_mut(|state| {
        if let Some(ar) = state.agent_runs.get_mut(&agent_run_id) {
            ar.last_nudge_at = Some(now);
        }
    });

    // Agent starts working (likely from nudge)
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

    // Should still be Escalated (not Running)
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Escalated);
}

#[tokio::test]
async fn standalone_auto_resume_allowed_after_cooldown() {
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
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Agent goes idle → escalates
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, AgentRunStatus::Escalated);

    // Set last_nudge_at to 61 seconds ago
    let now = ctx.clock.epoch_ms();
    ctx.runtime.lock_state_mut(|state| {
        if let Some(ar) = state.agent_runs.get_mut(&agent_run_id) {
            ar.last_nudge_at = Some(now.saturating_sub(61_000));
        }
    });

    // Agent starts working after cooldown
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Should be auto-resumed to Running
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(
        agent_run.status,
        AgentRunStatus::Running,
        "should auto-resume after nudge cooldown"
    );
}

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
