// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Auto-resume from escalation on Working state

use super::*;

// =============================================================================
// Job auto-resume from escalation on Working state
// =============================================================================

#[tokio::test]
async fn working_auto_resumes_job_from_waiting() {
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

    // Agent goes idle -> on_idle gate "false" fails -> job escalated to Waiting
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(job.step_status.is_waiting());

    // Agent starts working again (e.g., human typed in tmux or agent recovered)
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
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
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");
    assert_eq!(job.step_status, StepStatus::Running);

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // AgentWorking when already Running should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
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
async fn working_auto_resume_resets_action_attempts() {
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

    // Agent goes idle -> gate fails -> escalate -> Waiting
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Verify action attempts are non-empty after escalation
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());
    assert!(
        !job.action_tracker.action_attempts.is_empty(),
        "action_attempts should be non-empty after escalation"
    );

    // Agent starts working -> auto-resume
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Action attempts should be reset
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);
    assert!(
        job.action_tracker.action_attempts.is_empty(),
        "action_attempts should be cleared after auto-resume, got: {:?}",
        job.action_tracker.action_attempts
    );
}

// =============================================================================
// Standalone agent auto-resume from escalation
// =============================================================================

#[tokio::test]
async fn working_auto_resumes_standalone_agent_from_escalated() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_ESCALATE).await;

    // Spawn standalone agent via command
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

    // Find the agent_run and its agent_id
    let (agent_run_id, agent_id) = ctx.runtime.lock_state(|state| {
        let ar = state.agent_runs.values().next().unwrap();
        (ar.id.clone(), AgentId::new(ar.agent_id.clone().unwrap()))
    });

    // Verify agent run is Running
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Running);

    // Agent goes idle -> on_idle = escalate -> Escalated
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
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);

    // Agent starts working again -> should auto-resume to Running
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
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Running);
}

#[tokio::test]
async fn working_noop_when_standalone_agent_already_running() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_ESCALATE).await;

    // Spawn standalone agent
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

    let (agent_run_id, agent_id) = ctx.runtime.lock_state(|state| {
        let ar = state.agent_runs.values().next().unwrap();
        (ar.id.clone(), AgentId::new(ar.agent_id.clone().unwrap()))
    });

    // Verify already Running
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Running);

    // AgentWorking when already Running should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty());

    // Status should remain Running
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Running);
}

#[tokio::test]
async fn working_auto_resume_resets_standalone_action_attempts() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_ESCALATE).await;

    // Spawn standalone agent
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

    let (agent_run_id, agent_id) = ctx.runtime.lock_state(|state| {
        let ar = state.agent_runs.values().next().unwrap();
        (ar.id.clone(), AgentId::new(ar.agent_id.clone().unwrap()))
    });

    // Agent goes idle -> escalated
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Verify escalated and has action attempts
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);
    assert!(
        !agent_run.action_tracker.action_attempts.is_empty(),
        "action_attempts should be non-empty after escalation"
    );

    // Agent starts working -> auto-resume
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Action attempts should be cleared
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Running);
    assert!(
        agent_run.action_tracker.action_attempts.is_empty(),
        "action_attempts should be cleared after auto-resume, got: {:?}",
        agent_run.action_tracker.action_attempts
    );
}
