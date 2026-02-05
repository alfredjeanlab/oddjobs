// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Duplicate idle/prompt decision prevention and stale agent event filtering

use super::*;

// =============================================================================
// Duplicate idle/prompt decision prevention
// =============================================================================

#[tokio::test]
async fn duplicate_idle_creates_only_one_decision() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    // Set agent state so grace timer check confirms idle
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle -> sets grace timer (no immediate action)
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer -> escalate -> creates decision, sets step to Waiting
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(
        job.step_status.is_waiting(),
        "step should be waiting after first idle"
    );
    let decisions_after_first = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(
        decisions_after_first, 1,
        "should have exactly 1 decision after first idle"
    );

    // Second idle -> should be dropped (step already waiting, grace timer handler
    // checks job.step_status.is_waiting())
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Even if grace timer fires again, it should be no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "second idle should produce no events");
    let decisions_after_second = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(
        decisions_after_second, 1,
        "should still have exactly 1 decision after duplicate idle"
    );

    // Job should still be at work step, waiting
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(job.step_status.is_waiting());
}

#[tokio::test]
async fn prompt_hook_noop_when_step_already_waiting() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ESCALATE).await;

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

    // Set agent state so grace timer check confirms idle
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle -> sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer -> escalate -> step waiting
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());

    // Prompt event while step is already waiting -> should be dropped
    let result = ctx
        .runtime
        .handle_event(Event::AgentPrompt {
            agent_id: agent_id.clone(),
            prompt_type: oj_core::PromptType::Permission,
            question_data: None,
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "prompt should be dropped when step is already waiting"
    );
    let decisions = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(decisions, 1, "no additional decision should be created");
}

#[tokio::test]
async fn standalone_duplicate_idle_creates_only_one_escalation() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_ESCALATE).await;

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

    // Set agent state so grace timer check confirms idle
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle -> sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer -> escalated
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace_agent_run(&AgentRunId::new(&agent_run_id)),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);

    // Second idle -> should be dropped (already escalated)
    let result = ctx
        .runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "second idle should produce no events for escalated agent"
    );

    // Status should still be Escalated (not double-escalated)
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);
}

#[tokio::test]
async fn standalone_prompt_noop_when_agent_escalated() {
    let ctx = setup_with_runbook(RUNBOOK_AGENT_ESCALATE).await;

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

    // Set agent state so grace timer check confirms idle
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle -> sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer -> escalated
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace_agent_run(&AgentRunId::new(&agent_run_id)),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);

    // Prompt while escalated -> should be dropped
    let result = ctx
        .runtime
        .handle_event(Event::AgentPrompt {
            agent_id: agent_id.clone(),
            prompt_type: oj_core::PromptType::Permission,
            question_data: None,
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "prompt should be dropped when agent is escalated"
    );
}

// =============================================================================
// Stale agent event filtering
// =============================================================================

#[tokio::test]
async fn stale_agent_event_dropped_after_job_advances() {
    // Use the default TEST_RUNBOOK which has: init (shell) -> plan (agent) -> execute (agent)
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance past init (shell) to plan (agent)
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

    // Capture the old agent_id from the "plan" step
    let old_agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Advance from plan to execute (another agent step)
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "execute");

    let new_agent_id = get_agent_id(&ctx, &job_id).unwrap();
    assert_ne!(old_agent_id.as_str(), new_agent_id.as_str());

    // Send a stale AgentWaiting event from the OLD agent — should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWaiting {
            agent_id: old_agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty());

    // Job should still be at "execute", not affected by the stale event
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "execute");
}

#[tokio::test]
async fn stale_agent_signal_dropped_after_job_advances() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance past init (shell) to plan (agent)
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

    let old_agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Advance from plan to execute
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "execute");

    // Send a stale AgentSignal::Complete from the OLD agent — should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentSignal {
            agent_id: old_agent_id.clone(),
            kind: AgentSignalKind::Complete,
            message: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty());

    // Job should still be at "execute"
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "execute");
}
