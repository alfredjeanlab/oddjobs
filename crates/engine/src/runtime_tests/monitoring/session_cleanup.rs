// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session cleanup on agent signals and idle actions

use super::*;

// =============================================================================
// Standalone agent signal: session cleanup
// =============================================================================

#[tokio::test]
async fn standalone_agent_signal_complete_kills_session() {
    let ctx = setup_with_runbook(RUNBOOK_STANDALONE_AGENT).await;
    let (_agent_run_id, session_id, agent_id) = setup_standalone_agent(&ctx).await;

    // Register the session as alive
    ctx.sessions.add_session(&session_id, true);

    // Agent signals complete
    ctx.runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Complete,
            message: None,
        })
        .await
        .unwrap();

    // Verify the agent run status is Completed
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get("pipe-1").cloned())
        .unwrap();
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Completed);

    // Verify the session was killed
    let kills: Vec<_> = ctx
        .sessions
        .calls()
        .into_iter()
        .filter(|c| matches!(c, SessionCall::Kill { id } if id == &session_id))
        .collect();
    assert!(
        !kills.is_empty(),
        "session should be killed after agent:signal complete"
    );
}

#[tokio::test]
async fn standalone_agent_on_idle_done_kills_session() {
    let ctx = setup_with_runbook(RUNBOOK_STANDALONE_AGENT).await;
    let (_agent_run_id, session_id, agent_id) = setup_standalone_agent(&ctx).await;

    // Register the session as alive
    ctx.sessions.add_session(&session_id, true);

    // Agent goes idle — on_idle = "done" should complete the agent run
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Verify the agent run status is Completed
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get("pipe-1").cloned())
        .unwrap();
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Completed);

    // Verify the session was killed
    let kills: Vec<_> = ctx
        .sessions
        .calls()
        .into_iter()
        .filter(|c| matches!(c, SessionCall::Kill { id } if id == &session_id))
        .collect();
    assert!(
        !kills.is_empty(),
        "session should be killed after on_idle=done completes agent run"
    );
}

#[tokio::test]
async fn standalone_agent_signal_escalate_keeps_session() {
    let ctx = setup_with_runbook(RUNBOOK_STANDALONE_AGENT).await;
    let (_agent_run_id, session_id, agent_id) = setup_standalone_agent(&ctx).await;

    // Register the session as alive
    ctx.sessions.add_session(&session_id, true);

    // Agent signals escalate
    ctx.runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Escalate,
            message: Some("need help".to_string()),
        })
        .await
        .unwrap();

    // Verify the agent run status is Escalated (not terminal)
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get("pipe-1").cloned())
        .unwrap();
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);

    // Verify the session was NOT killed (agent stays alive for user interaction)
    let kills: Vec<_> = ctx
        .sessions
        .calls()
        .into_iter()
        .filter(|c| matches!(c, SessionCall::Kill { id } if id == &session_id))
        .collect();
    assert!(
        kills.is_empty(),
        "session should NOT be killed on escalate (agent stays alive)"
    );
}

// =============================================================================
// Job agent signal: session cleanup
// =============================================================================

#[tokio::test]
async fn job_agent_signal_complete_kills_session() {
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
    let job = ctx.runtime.get_job(&job_id).unwrap();
    let session_id = job.session_id.clone().unwrap();

    // Register the session as alive
    ctx.sessions.add_session(&session_id, true);

    // Agent signals complete — job should advance AND kill the session
    ctx.runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Complete,
            message: None,
        })
        .await
        .unwrap();

    // Job advanced
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");

    // Session was killed
    let kills: Vec<_> = ctx
        .sessions
        .calls()
        .into_iter()
        .filter(|c| matches!(c, SessionCall::Kill { id } if id == &session_id))
        .collect();
    assert!(
        !kills.is_empty(),
        "session should be killed when job agent signals complete"
    );
}
