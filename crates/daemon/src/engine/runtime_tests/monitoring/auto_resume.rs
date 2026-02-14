// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Auto-resume from escalation on Working state

use super::*;

#[tokio::test]
async fn working_auto_resumes_job_from_waiting() {
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Test\"\non_idle = { action = \"gate\", run = \"false\" }",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent goes idle -> on_idle gate "false" fails -> job escalated to Waiting
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(job.step_status.is_waiting());

    // Agent starts working again (e.g., human attached to session or agent recovered)
    ctx.runtime
        .handle_event(Event::AgentWorking {
            id: agent_id.clone(),
            owner: JobId::from_string(&job_id).into(),
        })
        .await
        .unwrap();

    // Job should be back to Running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(job.step_status, StepStatus::Running);
}

#[tokio::test]
async fn working_noop_when_job_already_running() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let job_id = create_job(&ctx).await;

    // Advance to agent step
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");
    assert_eq!(job.step_status, StepStatus::Running);

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // AgentWorking when already Running should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWorking {
            id: agent_id.clone(),
            owner: JobId::from_string(&job_id).into(),
        })
        .await
        .unwrap();

    assert!(result.is_empty());

    // Job should remain at same step with same status
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");
    assert_eq!(job.step_status, StepStatus::Running);
}

#[tokio::test]
async fn working_auto_resume_resets_attempts() {
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Test\"\non_idle = { action = \"gate\", run = \"false\" }",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent goes idle -> gate fails -> escalate -> Waiting
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    // Verify action attempts are non-empty after escalation
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());
    assert!(!job.actions.attempts.is_empty(), "attempts should be non-empty after escalation");

    // Agent starts working -> auto-resume
    ctx.runtime
        .handle_event(Event::AgentWorking {
            id: agent_id.clone(),
            owner: JobId::from_string(&job_id).into(),
        })
        .await
        .unwrap();

    // Action attempts should be reset
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);
    assert!(
        job.actions.attempts.is_empty(),
        "attempts should be cleared after auto-resume, got: {:?}",
        job.actions.attempts
    );
}

#[tokio::test]
async fn working_auto_resumes_standalone_agent_from_escalated() {
    let ctx = setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"")).await;

    // Spawn standalone agent via command
    handle_event_chain(
        &ctx,
        crew_command_event(
            "run-1",
            "worker",
            "agent_cmd",
            vars!("name" => "test"),
            &ctx.project_path,
        ),
    )
    .await;

    // Find the crew and its agent_id
    let (crew_id, agent_id) = ctx.runtime.lock_state(|state| {
        let run = state.crew.values().next().unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.clone().unwrap()))
    });

    // Verify crew is Running
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Running);

    // Agent goes idle -> on_idle = escalate -> Escalated
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Escalated);

    // Agent starts working again -> should auto-resume to Running
    ctx.runtime
        .handle_event(Event::AgentWorking {
            id: agent_id.clone(),
            owner: CrewId::from_string(&crew_id).into(),
        })
        .await
        .unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Running);
}

#[tokio::test]
async fn working_noop_when_standalone_agent_already_running() {
    let ctx = setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"")).await;

    // Spawn standalone agent
    handle_event_chain(
        &ctx,
        crew_command_event(
            "run-1",
            "worker",
            "agent_cmd",
            vars!("name" => "test"),
            &ctx.project_path,
        ),
    )
    .await;

    let (crew_id, agent_id) = ctx.runtime.lock_state(|state| {
        let run = state.crew.values().next().unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.clone().unwrap()))
    });

    // Verify already Running
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Running);

    // AgentWorking when already Running should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWorking {
            id: agent_id.clone(),
            owner: CrewId::from_string(&crew_id).into(),
        })
        .await
        .unwrap();

    assert!(result.is_empty());

    // Status should remain Running
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Running);
}

#[tokio::test]
async fn working_auto_resume_resets_standalone_attempts() {
    let ctx = setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"")).await;

    // Spawn standalone agent
    handle_event_chain(
        &ctx,
        crew_command_event(
            "run-1",
            "worker",
            "agent_cmd",
            vars!("name" => "test"),
            &ctx.project_path,
        ),
    )
    .await;

    let (crew_id, agent_id) = ctx.runtime.lock_state(|state| {
        let run = state.crew.values().next().unwrap();
        (run.id.clone(), AgentId::from_string(run.agent_id.clone().unwrap()))
    });

    // Agent goes idle -> escalated
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), CrewId::from_string(&crew_id).into()))
        .await
        .unwrap();

    // Verify escalated and has action attempts
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Escalated);
    assert!(!crew.actions.attempts.is_empty(), "attempts should be non-empty after escalation");

    // Agent starts working -> auto-resume
    ctx.runtime
        .handle_event(Event::AgentWorking {
            id: agent_id.clone(),
            owner: CrewId::from_string(&crew_id).into(),
        })
        .await
        .unwrap();

    // Action attempts should be cleared
    let crew = ctx.runtime.lock_state(|s| s.crew.get(&crew_id).cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Running);
    assert!(
        crew.actions.attempts.is_empty(),
        "attempts should be cleared after auto-resume, got: {:?}",
        crew.actions.attempts
    );
}
