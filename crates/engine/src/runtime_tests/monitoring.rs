// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent monitoring timer and event handler tests

use super::*;
use oj_adapters::SessionCall;
use oj_core::{AgentRunId, AgentSignalKind, JobId, StepStatus, TimerId};

/// Helper: create a job and advance it to the "plan" agent step.
///
/// Returns (job_id, session_id, agent_id).
async fn setup_job_at_agent_step(ctx: &TestContext) -> (String, String, AgentId) {
    let job_id = create_job(ctx).await;

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

    let session_id = job.session_id.clone().unwrap();
    let agent_id = get_agent_id(ctx, &job_id).unwrap();

    (job_id, session_id, agent_id)
}

// =============================================================================
// Liveness timer happy paths
// =============================================================================

#[tokio::test]
async fn liveness_timer_reschedules_when_session_alive() {
    let ctx = setup().await;
    let (job_id, session_id, _agent_id) = setup_job_at_agent_step(&ctx).await;

    // Register the session as alive in the fake adapter
    ctx.sessions.add_session(&session_id, true);

    // Fire the liveness timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::new(job_id.clone())),
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
    assert!(timer_ids.contains(&TimerId::liveness(&JobId::new(job_id.clone())).as_str()));
    assert!(!timer_ids.iter().any(|id| id.starts_with("exit-deferred:")));
}

#[tokio::test]
async fn liveness_timer_schedules_deferred_exit_when_session_dead() {
    let ctx = setup().await;
    let (job_id, _session_id, _agent_id) = setup_job_at_agent_step(&ctx).await;

    // Don't add the session to FakeSessionAdapter — is_alive returns false for unknown sessions

    // Fire the liveness timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::new(job_id.clone())),
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
    assert!(timer_ids.contains(&TimerId::exit_deferred(&JobId::new(job_id.clone())).as_str()));
}

// =============================================================================
// Deferred exit timer happy paths
// =============================================================================

#[tokio::test]
async fn exit_deferred_timer_noop_when_job_terminal() {
    let ctx = setup().await;
    let (job_id, _session_id, _agent_id) = setup_job_at_agent_step(&ctx).await;

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

    // Deferred exit on a terminal job should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

/// Runbook with agent on_idle = done, on_dead = done, on_error = "fail"
const RUNBOOK_MONITORING: &str = r#"
[command.build]
args = "<name> <prompt>"
run = { job = "build" }

[job.build]
input  = ["name", "prompt"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = "plan"

[[job.build.step]]
name = "plan"
run = { agent = "planner" }
on_done = "done"

[[job.build.step]]
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

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent state to WaitingForInput
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // With on_idle = done, job should advance past the agent step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

#[tokio::test]
async fn exit_deferred_timer_on_error_when_agent_failed() {
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

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent state to Failed
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Failed(oj_core::AgentError::Other("test error".to_string())),
    );

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // With on_error = fail, job should be failed
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed");
}

#[tokio::test]
async fn exit_deferred_timer_on_dead_for_exited_state() {
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

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent state to Exited (maps to on_dead fallback)
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Exited { exit_code: Some(0) },
    );

    // Fire the deferred exit timer
    let _result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // With on_dead = done, job should advance
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
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

    // Emit AgentWaiting (on_idle = done → advance)
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

/// Runbook with on_idle = gate (failing command)
const RUNBOOK_GATE_IDLE_FAIL: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]

[[job.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "done"

[[job.build.step]]
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

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent goes idle, on_idle gate runs "false" which fails → escalate
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
    // Gate failed → escalate → Waiting status (job does NOT advance)
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

    // Agent goes idle → on_idle gate "false" fails → job escalated to Waiting
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

// =============================================================================
// Auto-resume from escalation on Working state
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

    // Agent goes idle → on_idle gate "false" fails → job escalated to Waiting
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

    // Agent goes idle → gate fails → escalate → Waiting
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

    // Agent starts working → auto-resume
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

/// Runbook with standalone agent command, on_idle = escalate
const RUNBOOK_AGENT_ESCALATE: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = "escalate"

[job.build]
input = ["name"]

[[job.build.step]]
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
            owner: None,
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

    // Agent goes idle → escalated
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

    // Agent starts working → auto-resume
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

// =============================================================================
// Duplicate idle/prompt decision prevention
// =============================================================================

/// Runbook with job agent that escalates on idle
const RUNBOOK_JOB_ESCALATE: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[[job.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo done"

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = "escalate"
"#;

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

    // First idle → sets grace timer (no immediate action)
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer → escalate → creates decision, sets step to Waiting
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

    // Second idle → should be dropped (step already waiting, grace timer handler
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

    // First idle → sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer → escalate → step waiting
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());

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

    // Set agent state so grace timer check confirms idle
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle → sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer → escalated
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

    // Set agent state so grace timer check confirms idle
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // First idle → sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer → escalated
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
async fn stale_agent_event_dropped_after_job_advances() {
    // Use the default TEST_RUNBOOK which has: init (shell) → plan (agent) → execute (agent)
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

// =============================================================================
// Idle grace timer tests
// =============================================================================

/// AgentIdle sets a grace timer and records log size; doesn't immediately trigger on_idle.
#[tokio::test]
async fn idle_grace_timer_set_on_agent_idle() {
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

    // Set a known log size
    ctx.agents.set_session_log_size(&agent_id, Some(42));

    // AgentIdle should NOT immediately escalate
    let result = ctx
        .runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "AgentIdle should produce no immediate events (grace timer defers action)"
    );

    // Job should still be Running (not escalated)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);

    // Grace timer should be scheduled
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
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
        timer_ids.iter().any(|id| id.starts_with("idle-grace:")),
        "idle grace timer should be scheduled, found: {:?}",
        timer_ids
    );

    // Log size should be recorded on the job
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.idle_grace_log_size, Some(42));
}

/// Second AgentIdle while grace timer is pending is a no-op (deduplication).
#[tokio::test]
async fn idle_grace_timer_deduplicates() {
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

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // First AgentIdle → sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.idle_grace_log_size, Some(100));

    // Increase log size to simulate activity
    ctx.agents.set_session_log_size(&agent_id, Some(200));

    // Second AgentIdle → should be deduplicated (idle_grace_log_size already set)
    let result = ctx
        .runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "duplicate AgentIdle should be no-op");

    // Log size should NOT be updated (still 100 from first idle)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.idle_grace_log_size, Some(100));
}

/// Working state cancels pending idle grace timer.
#[tokio::test]
async fn idle_grace_timer_cancelled_on_working() {
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

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // AgentIdle → sets grace timer
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.idle_grace_log_size.is_some());

    // AgentWorking → should cancel grace timer and clear log size
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.idle_grace_log_size, None,
        "idle_grace_log_size should be cleared on Working"
    );
}

/// Grace timer fires but log grew → no action (agent was active during grace period).
#[tokio::test]
async fn idle_grace_timer_noop_when_log_grew() {
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

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // AgentIdle → sets grace timer, records log_size=100
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Simulate log growth during grace period
    ctx.agents.set_session_log_size(&agent_id, Some(200));
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // Fire the grace timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "grace timer should produce no events when log grew"
    );

    // Job should still be Running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);
}

/// Grace timer fires, log unchanged but agent is Working → no action (race guard).
#[tokio::test]
async fn idle_grace_timer_noop_when_agent_working() {
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

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // AgentIdle → sets grace timer, records log_size=100
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Log hasn't grown, but agent started working (race condition guard)
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::Working);

    // Fire the grace timer
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "grace timer should produce no events when agent is Working"
    );

    // Job should still be Running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);
}

/// Grace timer fires, log unchanged + agent WaitingForInput → proceeds with on_idle.
#[tokio::test]
async fn idle_grace_timer_proceeds_when_genuinely_idle() {
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

    ctx.agents.set_session_log_size(&agent_id, Some(100));
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);

    // AgentIdle → sets grace timer, records log_size=100
    ctx.runtime
        .handle_event(Event::AgentIdle {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Fire the grace timer — log unchanged, agent idle → should proceed with on_idle
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::idle_grace(&JobId::new(job_id.clone())),
        })
        .await
        .unwrap();

    // on_idle = escalate → job should be Waiting
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(
        job.step_status.is_waiting(),
        "job should be Waiting after genuine idle triggers on_idle=escalate"
    );
}

/// Working state within 60s of nudge doesn't auto-resume or reset attempts.
#[tokio::test]
async fn auto_resume_suppressed_after_nudge() {
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

    // Put job into Waiting state via AgentWaiting (direct monitor path)
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
    assert!(job.step_status.is_waiting());

    // Simulate a nudge having been sent recently by setting last_nudge_at
    let now = ctx.clock.epoch_ms();
    let pid = JobId::new(&job_id);
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(pid.as_str()) {
            p.last_nudge_at = Some(now);
        }
    });

    // Agent starts working (likely from our nudge text)
    let result = ctx
        .runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "auto-resume should be suppressed within 60s of nudge"
    );

    // Job should still be Waiting (not resumed to Running)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(
        job.step_status.is_waiting(),
        "job should remain Waiting when Working is suppressed after nudge"
    );
}

/// Working state after 60s of nudge allows normal auto-resume.
#[tokio::test]
async fn auto_resume_allowed_after_nudge_cooldown() {
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

    // Put job into Waiting state
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
    assert!(job.step_status.is_waiting());

    // Set last_nudge_at to 61 seconds ago
    let now = ctx.clock.epoch_ms();
    let pid = JobId::new(&job_id);
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(pid.as_str()) {
            p.last_nudge_at = Some(now.saturating_sub(61_000));
        }
    });

    // Agent starts working after cooldown period
    ctx.runtime
        .handle_event(Event::AgentWorking {
            agent_id: agent_id.clone(),
            owner: None,
        })
        .await
        .unwrap();

    // Job should be auto-resumed to Running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.step_status,
        StepStatus::Running,
        "job should auto-resume after nudge cooldown expires"
    );
}

/// Rapid AgentIdle/Working cycling (simulating inter-tool-call gaps) never triggers nudge.
#[tokio::test]
async fn rapid_idle_working_cycling_no_nudge() {
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

    ctx.agents.set_session_log_size(&agent_id, Some(100));

    // Simulate 5 rapid idle/working cycles (like between tool calls)
    for i in 0..5 {
        ctx.agents
            .set_session_log_size(&agent_id, Some(100 + i * 50));

        // AgentIdle → sets grace timer
        ctx.runtime
            .handle_event(Event::AgentIdle {
                agent_id: agent_id.clone(),
            })
            .await
            .unwrap();

        // AgentWorking → cancels grace timer
        ctx.runtime
            .handle_event(Event::AgentWorking {
                agent_id: agent_id.clone(),
                owner: None,
            })
            .await
            .unwrap();
    }

    // Job should still be Running — no escalation happened
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(
        job.step_status,
        StepStatus::Running,
        "rapid idle/working cycling should never trigger on_idle"
    );
    assert_eq!(
        job.idle_grace_log_size, None,
        "grace log size should be cleared after Working"
    );
}
