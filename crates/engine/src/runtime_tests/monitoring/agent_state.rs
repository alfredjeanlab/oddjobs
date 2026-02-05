// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent state event handling, gate/signal interactions, and signal continue tests

use super::*;

// =============================================================================
// Agent state event handling
// =============================================================================

#[tokio::test]
async fn agent_state_changed_unknown_agent_is_noop() {
    let ctx = setup().await;

    let result = ctx
        .runtime
        .handle_event(Event::AgentWaiting {
            agent_id: AgentId::new("unknown-agent".to_string()),
            owner: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_state_changed_terminal_job_is_noop() {
    let ctx = setup().await;
    let (job_id, _session_id, agent_id) = setup_job_at_agent_step(&ctx).await;

    // Fail the job to make it terminal
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "plan".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.is_terminal());

    // AgentWaiting for terminal job should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWaiting {
            agent_id,
            owner: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_state_changed_routes_through_agent_jobs() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let job_id = create_job(&ctx).await;

    // Advance to agent step (which calls spawn_agent, populating agent_jobs)
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

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Emit AgentWaiting (on_idle = done -> advance)
    let _result = ctx
        .runtime
        .handle_event(Event::AgentWaiting {
            agent_id,
            owner: None,
        })
        .await
        .unwrap();

    // on_idle = done should advance the job
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

// =============================================================================
// on_idle gate + agent:signal complete interaction
// =============================================================================

#[tokio::test]
async fn gate_idle_escalates_when_command_fails() {
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
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent goes idle, on_idle gate runs "false" which fails -> escalate
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
    // Gate failed -> escalate -> Waiting status (job does NOT advance)
    assert_eq!(job.step, "work");
    assert!(job.step_status.is_waiting());
}

#[tokio::test]
async fn agent_signal_complete_overrides_gate_escalation() {
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

    // Agent signals complete — this should override the gate escalation
    let result = ctx
        .runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Complete,
            message: None,
        })
        .await
        .unwrap();

    // Signal should produce events (job advances)
    assert!(!result.is_empty());

    // Job should have advanced past "work" to "done"
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

#[tokio::test]
async fn agent_signal_complete_advances_job() {
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

    // Job is at "work" step, agent is running (no idle yet)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(job.step_status, StepStatus::Running);

    // Agent signals complete before going idle — job advances immediately
    let result = ctx
        .runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Complete,
            message: None,
        })
        .await
        .unwrap();

    assert!(!result.is_empty());

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

// =============================================================================
// agent:signal continue — no-op
// =============================================================================

#[tokio::test]
async fn agent_signal_continue_no_job_state_change() {
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

    // Job is at "work" step, agent is running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(job.step_status, StepStatus::Running);

    // Agent signals continue — should be a no-op (no state change)
    let result = ctx
        .runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Continue,
            message: None,
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "continue signal should produce no events"
    );

    // Job should remain at same step with same status
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(job.step_status, StepStatus::Running);
}

#[tokio::test]
async fn standalone_agent_signal_continue_no_state_change() {
    let ctx = setup_with_runbook(RUNBOOK_STANDALONE_AGENT).await;
    let (agent_run_id, _session_id, agent_id) = setup_standalone_agent(&ctx).await;

    // Verify agent run is Running
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Running);

    // Agent signals continue — should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Continue,
            message: None,
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "continue signal should produce no events for standalone agent"
    );

    // Status should remain Running
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Running);
}
