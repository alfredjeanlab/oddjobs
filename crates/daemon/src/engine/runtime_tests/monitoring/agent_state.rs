// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent state event handling, gate/signal interactions, and signal continue tests

use super::*;

#[tokio::test]
async fn agent_state_changed_unknown_agent_is_noop() {
    let ctx = setup().await;

    let result = ctx
        .runtime
        .handle_event(agent_waiting(
            AgentId::from_string("unknown-agent".to_string()),
            OwnerId::Job(JobId::default()),
        ))
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_state_changed_terminal_job_is_noop() {
    let mut ctx = setup().await;
    let (job_id, agent_id) = setup_job_at_agent_step(&mut ctx).await;

    // Fail the job to make it terminal
    ctx.runtime.handle_event(shell_fail(&job_id, "plan")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.is_terminal());

    // AgentWaiting for terminal job should be a no-op
    let result = ctx
        .runtime
        .handle_event(agent_waiting(agent_id, JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_state_changed_routes_through_agent_jobs() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let job_id = create_job(&ctx).await;

    // Advance to agent step (which calls spawn_agent, populating agent_jobs)
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Emit AgentWaiting (on_idle = done -> advance)
    let _result = ctx
        .runtime
        .handle_event(agent_waiting(agent_id, JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    // on_idle = done should advance the job
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

#[tokio::test]
async fn gate_idle_escalates_when_command_fails() {
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Test\"\non_idle = { action = \"gate\", run = \"false\" }",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent goes idle, on_idle gate runs "false" which fails -> escalate
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    // Gate failed -> escalate -> Waiting status (job does NOT advance)
    assert_eq!(job.step, "work");
    assert!(job.step_status.is_waiting());
}
