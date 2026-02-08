// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session cleanup on agent signals and idle actions

use super::*;

// =============================================================================
// Standalone agent signal: session cleanup
// =============================================================================

#[tokio::test]
async fn standalone_agent_signal_complete_kills_session() {
    let mut ctx = setup_with_runbook(RUNBOOK_STANDALONE_AGENT).await;
    let (_agent_run_id, session_id, agent_id) = setup_standalone_agent(&mut ctx).await;

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

    // Yield to let fire-and-forget KillSession task complete
    tokio::task::yield_now().await;

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
    let mut ctx = setup_with_runbook(RUNBOOK_STANDALONE_AGENT).await;
    let (_agent_run_id, session_id, agent_id) = setup_standalone_agent(&mut ctx).await;

    // Register the session as alive
    ctx.sessions.add_session(&session_id, true);

    // Agent goes idle — on_idle = "done" should complete the agent run
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: OwnerId::AgentRun(AgentRunId::new("pipe-1")),
        })
        .await
        .unwrap();

    // Verify the agent run status is Completed
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get("pipe-1").cloned())
        .unwrap();
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Completed);

    // Yield to let fire-and-forget KillSession task complete
    tokio::task::yield_now().await;

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
    let mut ctx = setup_with_runbook(RUNBOOK_STANDALONE_AGENT).await;
    let (_agent_run_id, session_id, agent_id) = setup_standalone_agent(&mut ctx).await;

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

    // Yield to let any fire-and-forget tasks complete before checking
    tokio::task::yield_now().await;

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
    let mut ctx = setup_with_runbook(RUNBOOK_GATE_IDLE_FAIL).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;
    ctx.process_background_events().await;

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

    // Yield to let fire-and-forget KillSession task complete
    tokio::task::yield_now().await;

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

#[tokio::test]
async fn job_agent_signal_escalate_creates_decision() {
    let mut ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;
    ctx.process_background_events().await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    let job = ctx.runtime.get_job(&job_id).unwrap();
    let session_id = job.session_id.clone().unwrap();

    ctx.sessions.add_session(&session_id, true);

    // Agent signals escalate
    ctx.runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Escalate,
            message: Some("Need help with merge conflicts".to_string()),
        })
        .await
        .unwrap();

    // Job step should be waiting with a decision_id
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(
        matches!(job.step_status, StepStatus::Waiting(Some(_))),
        "step should be Waiting with decision_id, got {:?}",
        job.step_status
    );

    // A decision should exist
    let decision_count = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(
        decision_count, 1,
        "escalate signal should create a decision"
    );
    let decision = ctx
        .runtime
        .lock_state(|s| s.decisions.values().next().cloned().unwrap());
    assert_eq!(decision.source, oj_core::DecisionSource::Signal);
    assert!(!decision.is_resolved());
    assert!(decision.context.contains("Need help with merge conflicts"));

    // Yield to let any fire-and-forget tasks complete before checking
    tokio::task::yield_now().await;

    // Session should NOT be killed (agent stays alive)
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

#[tokio::test]
async fn job_agent_signal_escalate_default_message() {
    let mut ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;
    ctx.process_background_events().await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    let job = ctx.runtime.get_job(&job_id).unwrap();
    let session_id = job.session_id.clone().unwrap();

    ctx.sessions.add_session(&session_id, true);

    // Agent signals escalate without a message
    ctx.runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Escalate,
            message: None,
        })
        .await
        .unwrap();

    // Decision should use default message
    let decision = ctx
        .runtime
        .lock_state(|s| s.decisions.values().next().cloned().unwrap());
    assert!(decision.context.contains("Agent requested escalation"));
}
