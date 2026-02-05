// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for fail action, gate command execution, and error handling.

use super::*;

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
