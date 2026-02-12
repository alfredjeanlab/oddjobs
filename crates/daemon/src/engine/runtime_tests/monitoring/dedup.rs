// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Duplicate idle/prompt decision prevention and stale agent event filtering

use super::*;

#[tokio::test]
async fn duplicate_idle_creates_only_one_decision() {
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Do the work\"\non_idle = \"escalate\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // First stop:blocked → escalate → creates decision, sets step to Waiting
    ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting(), "step should be waiting after first idle");
    let decisions_after_first = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(decisions_after_first, 1, "should have exactly 1 decision after first idle");

    // Second stop:blocked → should be dropped (step already waiting)
    let result =
        ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

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
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Do the work\"\non_idle = \"escalate\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // First stop:blocked → escalate → step waiting
    ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());

    // Prompt event while step is already waiting -> should be dropped
    let result = ctx
        .runtime
        .handle_event(Event::AgentPrompt {
            id: agent_id.clone(),
            prompt_type: oj_core::PromptType::Permission,
            questions: None,
            last_message: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "prompt should be dropped when step is already waiting");
    let decisions = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(decisions, 1, "no additional decision should be created");
}

#[tokio::test]
async fn standalone_duplicate_idle_creates_only_one_escalation() {
    let ctx = setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"")).await;

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

    let (_crew_id, agent_id) = ctx.runtime.lock_state(|state| {
        let run = state.crew.values().next().unwrap();
        (run.id.clone(), AgentId::new(run.agent_id.clone().unwrap()))
    });

    // First stop:blocked → escalated
    ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get("run-1").cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Escalated);

    // Second stop:blocked → should be dropped (already escalated)
    let result =
        ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    assert!(result.is_empty(), "second idle should produce no events for escalated agent");

    // Status should still be Escalated (not double-escalated)
    let crew = ctx.runtime.lock_state(|s| s.crew.get("run-1").cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Escalated);
}

#[tokio::test]
async fn standalone_prompt_noop_when_agent_escalated() {
    let ctx = setup_with_runbook(&test_runbook_agent("on_idle = \"escalate\"")).await;

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

    let (_crew_id, agent_id) = ctx.runtime.lock_state(|state| {
        let run = state.crew.values().next().unwrap();
        (run.id.clone(), AgentId::new(run.agent_id.clone().unwrap()))
    });

    // First stop:blocked → escalated
    ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    let crew = ctx.runtime.lock_state(|s| s.crew.get("run-1").cloned().unwrap());
    assert_eq!(crew.status, oj_core::CrewStatus::Escalated);

    // Prompt while escalated -> should be dropped
    let result = ctx
        .runtime
        .handle_event(Event::AgentPrompt {
            id: agent_id.clone(),
            prompt_type: oj_core::PromptType::Permission,
            questions: None,
            last_message: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "prompt should be dropped when agent is escalated");
}

#[tokio::test]
async fn prompt_fires_without_exhaustion_after_resolution() {
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Do the work\"\non_idle = \"escalate\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // First prompt -> escalate -> creates decision, step Waiting
    ctx.runtime
        .handle_event(Event::AgentPrompt {
            id: agent_id.clone(),
            prompt_type: oj_core::PromptType::PlanApproval,
            questions: None,
            last_message: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting(), "step should be waiting after first prompt");
    let decisions_after_first = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(decisions_after_first, 1);

    // Simulate decision resolution + job resume: set step back to Running
    ctx.runtime.lock_state_mut(|state| {
        if let Some(j) = state.jobs.get_mut(&job_id) {
            j.step_status = StepStatus::Running;
        }
    });

    // Second prompt -> should create a new decision (not exhaust)
    let result = ctx
        .runtime
        .handle_event(Event::AgentPrompt {
            id: agent_id.clone(),
            prompt_type: oj_core::PromptType::PlanApproval,
            questions: None,
            last_message: None,
        })
        .await
        .unwrap();

    assert!(!result.is_empty(), "second prompt should produce events");

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting(), "step should be waiting after second prompt");
    let decisions_after_second = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(decisions_after_second, 2, "should have 2 decisions (one per prompt occurrence)");
}

#[tokio::test]
async fn stale_agent_event_dropped_after_job_advances() {
    // Use the default TEST_RUNBOOK which has: init (shell) -> plan (agent) -> execute (agent)
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance past init (shell) to plan (agent)
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

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
        .handle_event(agent_waiting(old_agent_id.clone(), JobId::new(&job_id).into()))
        .await
        .unwrap();

    assert!(result.is_empty());

    // Job should still be at "execute", not affected by the stale event
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "execute");
}
