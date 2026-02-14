// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session cleanup on stop-blocked events and idle actions

use super::*;

#[tokio::test]
async fn standalone_agent_stop_blocked_completes_and_kills_session() {
    let mut ctx =
        setup_with_runbook(&test_runbook_agent("on_idle = \"done\"\non_dead = \"done\"")).await;
    let (_crew_id, agent_id) = setup_standalone_agent(&mut ctx).await;

    // Agent stop blocked (coop's stop:outcome blocked)
    ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    // Verify the crew status is Completed
    let crew = ctx.runtime.lock_state(|s| s.crew.get("crw-1").cloned()).unwrap();
    assert_eq!(crew.status, oj_core::CrewStatus::Completed);

    // Yield to let fire-and-forget KillAgent task complete
    tokio::task::yield_now().await;

    // Verify the session was killed
    let kills: Vec<_> = ctx
        .agents
        .calls()
        .into_iter()
        .filter(|c| matches!(c, AgentCall::Kill { agent_id: aid } if *aid == agent_id))
        .collect();
    assert!(!kills.is_empty(), "session should be killed after agent:stop:blocked auto-complete");
}

#[tokio::test]
async fn standalone_agent_on_idle_done_kills_session() {
    let mut ctx =
        setup_with_runbook(&test_runbook_agent("on_idle = \"done\"\non_dead = \"done\"")).await;
    let (_crew_id, agent_id) = setup_standalone_agent(&mut ctx).await;

    // Agent goes idle — on_idle = "done" should complete the crew
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string("job-1").into()))
        .await
        .unwrap();

    // Verify the crew status is Completed
    let crew = ctx.runtime.lock_state(|s| s.crew.get("crw-1").cloned()).unwrap();
    assert_eq!(crew.status, oj_core::CrewStatus::Completed);

    // Yield to let fire-and-forget KillAgent task complete
    tokio::task::yield_now().await;

    // Verify the session was killed
    let kills: Vec<_> = ctx
        .agents
        .calls()
        .into_iter()
        .filter(|c| matches!(c, AgentCall::Kill { agent_id: aid } if *aid == agent_id))
        .collect();
    assert!(!kills.is_empty(), "session should be killed after on_idle=done completes crew");
}

#[tokio::test]
async fn job_agent_stop_blocked_completes_and_kills_session() {
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Test\"\non_idle = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent stop blocked — job should advance AND kill the session
    ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    // Job advanced
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");

    // Yield to let fire-and-forget KillAgent task complete
    tokio::task::yield_now().await;

    // Session was killed
    let kills: Vec<_> = ctx
        .agents
        .calls()
        .into_iter()
        .filter(|c| matches!(c, AgentCall::Kill { agent_id: aid } if *aid == agent_id))
        .collect();
    assert!(!kills.is_empty(), "session should be killed when job agent stop is blocked");
}
