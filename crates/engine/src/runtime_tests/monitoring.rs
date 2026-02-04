// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent monitoring timer and event handler tests

use super::*;
use oj_adapters::SessionCall;
use oj_core::{AgentSignalKind, PipelineId, StepStatus, TimerId};

/// Helper: create a pipeline and advance it to the "plan" agent step.
///
/// Returns (pipeline_id, session_id, agent_id).
async fn setup_pipeline_at_agent_step(ctx: &TestContext) -> (String, String, AgentId) {
    let pipeline_id = create_pipeline(ctx).await;

    // Advance past init (shell) to plan (agent)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    let session_id = pipeline.session_id.clone().unwrap();
    let agent_id = get_agent_id(ctx, &pipeline_id).unwrap();

    (pipeline_id, session_id, agent_id)
}

// =============================================================================
// Liveness timer happy paths
// =============================================================================

#[tokio::test]
async fn liveness_timer_reschedules_when_session_alive() {
    let ctx = setup().await;
    let (pipeline_id, session_id, _agent_id) = setup_pipeline_at_agent_step(&ctx).await;

    // Register the session as alive in the fake adapter
    ctx.sessions.add_session(&session_id, true);

    // Fire the liveness timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // Liveness check when alive produces no events (just reschedules the timer)
    assert!(result.is_empty());

    // Verify the liveness timer was rescheduled (not an exit-deferred timer)
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    assert!(sched.has_timers());
    ctx.clock.advance(std::time::Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(timer_ids.contains(&TimerId::liveness(&PipelineId::new(pipeline_id.clone())).as_str()));
    assert!(!timer_ids.iter().any(|id| id.starts_with("exit-deferred:")));
}

#[tokio::test]
async fn liveness_timer_schedules_deferred_exit_when_session_dead() {
    let ctx = setup().await;
    let (pipeline_id, _session_id, _agent_id) = setup_pipeline_at_agent_step(&ctx).await;

    // Don't add the session to FakeSessionAdapter — is_alive returns false for unknown sessions

    // Fire the liveness timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // Dead session produces no direct events (schedules deferred exit timer)
    assert!(result.is_empty());

    // Verify a deferred exit timer was scheduled
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    assert!(sched.has_timers());
    ctx.clock.advance(std::time::Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        timer_ids.contains(&TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())).as_str())
    );
}

// =============================================================================
// Deferred exit timer happy paths
// =============================================================================

#[tokio::test]
async fn exit_deferred_timer_noop_when_pipeline_terminal() {
    let ctx = setup().await;
    let (pipeline_id, _session_id, _agent_id) = setup_pipeline_at_agent_step(&ctx).await;

    // Fail the pipeline to make it terminal
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "plan".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.is_terminal());

    // Deferred exit on a terminal pipeline should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

/// Runbook with agent on_idle = done, on_dead = done, on_error = "fail"
const RUNBOOK_MONITORING: &str = r#"
[command.build]
args = "<name> <prompt>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name", "prompt"]

[[pipeline.build.step]]
name = "init"
run = "echo init"
on_done = "plan"

[[pipeline.build.step]]
name = "plan"
run = { agent = "planner" }
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[agent.planner]
run = "claude --print"
on_idle = "done"
on_dead = "done"
on_error = "fail"
"#;

#[tokio::test]
async fn exit_deferred_timer_on_idle_when_waiting_for_input() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to agent step
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Set agent state to WaitingForInput
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // With on_idle = done, pipeline should advance past the agent step
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
}

#[tokio::test]
async fn exit_deferred_timer_on_error_when_agent_failed() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to agent step
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Set agent state to Failed
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Failed(oj_core::AgentError::Other("test error".to_string())),
    );

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // With on_error = fail, pipeline should be failed
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "failed");
}

#[tokio::test]
async fn exit_deferred_timer_on_dead_for_exited_state() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to agent step
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Set agent state to Exited (maps to on_dead fallback)
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Exited { exit_code: Some(0) },
    );

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // With on_dead = done, pipeline should advance
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
}

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
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_state_changed_terminal_pipeline_is_noop() {
    let ctx = setup().await;
    let (pipeline_id, _session_id, agent_id) = setup_pipeline_at_agent_step(&ctx).await;

    // Fail the pipeline to make it terminal
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "plan".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.is_terminal());

    // AgentWaiting for terminal pipeline should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWaiting { agent_id })
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_state_changed_routes_through_agent_pipelines() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to agent step (which calls spawn_agent, populating agent_pipelines)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Emit AgentWaiting (on_idle = done → advance)
    let _result = ctx
        .runtime
        .handle_event(Event::AgentWaiting { agent_id })
        .await
        .unwrap();

    // on_idle = done should advance the pipeline
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
}

// =============================================================================
// on_idle gate + agent:signal complete interaction
// =============================================================================

/// Runbook with on_idle = gate (failing command)
const RUNBOOK_GATE_IDLE_FAIL: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[agent.worker]
run = 'claude'
prompt = "Test"
on_idle = { action = "gate", run = "false" }
"#;

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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Agent goes idle, on_idle gate runs "false" which fails → escalate
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    // Gate failed → escalate → Waiting status (pipeline does NOT advance)
    assert_eq!(pipeline.step, "work");
    assert!(pipeline.step_status.is_waiting());
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Agent goes idle → on_idle gate "false" fails → pipeline escalated to Waiting
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");
    assert!(pipeline.step_status.is_waiting());

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

    // Signal should produce events (pipeline advances)
    assert!(!result.is_empty());

    // Pipeline should have advanced past "work" to "done"
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
}

#[tokio::test]
async fn agent_signal_complete_advances_pipeline() {
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Pipeline is at "work" step, agent is running (no idle yet)
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");
    assert_eq!(pipeline.step_status, StepStatus::Running);

    // Agent signals complete before going idle — pipeline advances immediately
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

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
}

// =============================================================================
// Auto-resume from escalation on Working state
// =============================================================================

#[tokio::test]
async fn working_auto_resumes_pipeline_from_waiting() {
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Agent goes idle → on_idle gate "false" fails → pipeline escalated to Waiting
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");
    assert!(pipeline.step_status.is_waiting());

    // Agent starts working again (e.g., human typed in tmux or agent recovered)
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Pipeline should be back to Running
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");
    assert_eq!(pipeline.step_status, StepStatus::Running);
}

#[tokio::test]
async fn working_noop_when_pipeline_already_running() {
    let ctx = setup_with_runbook(RUNBOOK_MONITORING).await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to agent step
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");
    assert_eq!(pipeline.step_status, StepStatus::Running);

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // AgentWorking when already Running should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(result.is_empty());

    // Pipeline should remain at same step with same status
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");
    assert_eq!(pipeline.step_status, StepStatus::Running);
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Agent goes idle → gate fails → escalate → Waiting
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Verify action attempts are non-empty after escalation
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.step_status.is_waiting());
    assert!(
        !pipeline.action_attempts.is_empty(),
        "action_attempts should be non-empty after escalation"
    );

    // Agent starts working → auto-resume
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Action attempts should be reset
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step_status, StepStatus::Running);
    assert!(
        pipeline.action_attempts.is_empty(),
        "action_attempts should be cleared after auto-resume, got: {:?}",
        pipeline.action_attempts
    );
}

// =============================================================================
// Standalone agent auto-resume from escalation
// =============================================================================

/// Runbook with standalone agent command, on_idle = escalate
const RUNBOOK_AGENT_ESCALATE: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = "escalate"

[pipeline.build]
input = ["name"]

[[pipeline.build.step]]
name = "init"
run = "echo init"
"#;

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

    // Agent goes idle → on_idle = escalate → Escalated
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);

    // Agent starts working again → should auto-resume to Running
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
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

    // Agent goes idle → escalated
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Verify escalated and has action attempts
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);
    assert!(
        !agent_run.action_attempts.is_empty(),
        "action_attempts should be non-empty after escalation"
    );

    // Agent starts working → auto-resume
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Action attempts should be cleared
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Running);
    assert!(
        agent_run.action_attempts.is_empty(),
        "action_attempts should be cleared after auto-resume, got: {:?}",
        agent_run.action_attempts
    );
}

// =============================================================================
// Duplicate idle/prompt decision prevention
// =============================================================================

/// Runbook with pipeline agent that escalates on idle
const RUNBOOK_PIPELINE_ESCALATE: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input = ["name"]

[[pipeline.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = "escalate"
"#;

#[tokio::test]
async fn duplicate_idle_creates_only_one_decision() {
    let ctx = setup_with_runbook(RUNBOOK_PIPELINE_ESCALATE).await;

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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // First idle → escalate → creates decision, sets step to Waiting
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(
        pipeline.step_status.is_waiting(),
        "step should be waiting after first idle"
    );
    let decisions_after_first = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(
        decisions_after_first, 1,
        "should have exactly 1 decision after first idle"
    );

    // Second idle → should be dropped (step already waiting)
    let result = ctx
        .runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "second idle should produce no events");
    let decisions_after_second = ctx.runtime.lock_state(|s| s.decisions.len());
    assert_eq!(
        decisions_after_second, 1,
        "should still have exactly 1 decision after duplicate idle"
    );

    // Pipeline should still be at work step, waiting
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");
    assert!(pipeline.step_status.is_waiting());
}

#[tokio::test]
async fn prompt_hook_noop_when_step_already_waiting() {
    let ctx = setup_with_runbook(RUNBOOK_PIPELINE_ESCALATE).await;

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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // First idle → escalate → step waiting
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.step_status.is_waiting());

    // Prompt event while step is already waiting → should be dropped
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

    // First idle → escalated
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);

    // Second idle → should be dropped (already escalated)
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

    // First idle → escalated
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get(&agent_run_id).cloned().unwrap());
    assert_eq!(agent_run.status, oj_core::AgentRunStatus::Escalated);

    // Prompt while escalated → should be dropped
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
async fn stale_agent_event_dropped_after_pipeline_advances() {
    // Use the default TEST_RUNBOOK which has: init (shell) → plan (agent) → execute (agent)
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance past init (shell) to plan (agent)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    // Capture the old agent_id from the "plan" step
    let old_agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Advance from plan to execute (another agent step)
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "execute");

    let new_agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();
    assert_ne!(old_agent_id.as_str(), new_agent_id.as_str());

    // Send a stale AgentWaiting event from the OLD agent — should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentWaiting {
            agent_id: old_agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(result.is_empty());

    // Pipeline should still be at "execute", not affected by the stale event
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "execute");
}

#[tokio::test]
async fn stale_agent_signal_dropped_after_pipeline_advances() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance past init (shell) to plan (agent)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    let old_agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Advance from plan to execute
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "execute");

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

    // Pipeline should still be at "execute"
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "execute");
}

// =============================================================================
// Standalone agent signal: session cleanup
// =============================================================================

/// Runbook with a standalone agent command and on_idle = "done"
const RUNBOOK_STANDALONE_AGENT: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Hello"
on_idle = "done"
on_dead = "done"
"#;

/// Helper: spawn a standalone agent and return (agent_run_id, session_id, agent_id)
async fn setup_standalone_agent(ctx: &TestContext) -> (String, String, AgentId) {
    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "agent_cmd",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let agent_run_id = "pipe-1".to_string();
    let agent_run = ctx
        .runtime
        .lock_state(|s| s.agent_runs.get("pipe-1").cloned())
        .unwrap();
    let agent_id = AgentId::new(agent_run.agent_id.as_ref().unwrap());
    let session_id = agent_run.session_id.clone().unwrap();

    (agent_run_id, session_id, agent_id)
}

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
// Pipeline agent signal: session cleanup
// =============================================================================

#[tokio::test]
async fn pipeline_agent_signal_complete_kills_session() {
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    let session_id = pipeline.session_id.clone().unwrap();

    // Register the session as alive
    ctx.sessions.add_session(&session_id, true);

    // Agent signals complete — pipeline should advance AND kill the session
    ctx.runtime
        .handle_event(Event::AgentSignal {
            agent_id: agent_id.clone(),
            kind: AgentSignalKind::Complete,
            message: None,
        })
        .await
        .unwrap();

    // Pipeline advanced
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");

    // Session was killed
    let kills: Vec<_> = ctx
        .sessions
        .calls()
        .into_iter()
        .filter(|c| matches!(c, SessionCall::Kill { id } if id == &session_id))
        .collect();
    assert!(
        !kills.is_empty(),
        "session should be killed when pipeline agent signals complete"
    );
}
