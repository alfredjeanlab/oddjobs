// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for signal handling, nudge timestamp tracking, and auto-resume suppression.

use super::*;

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
