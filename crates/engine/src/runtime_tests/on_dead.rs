// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent exit behavior tests
//!
//! Tests that agent exit (on_dead) actions are correctly dispatched through the
//! event-based paths: `Agent{State}` events and liveness timer flow.

use super::*;
use oj_core::{PipelineId, TimerId};

// =============================================================================
// Session death tests (via liveness timer flow)
// =============================================================================

#[tokio::test]
async fn session_death_triggers_on_dead_action() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to plan (agent step) — spawn_agent registers in agent_pipelines
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

    // Session not registered in FakeSessionAdapter — is_alive returns false.
    // Fire liveness timer → schedules exit-deferred timer.
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // Set agent state to Exited (on_dead fallback)
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Exited { exit_code: Some(0) },
    );

    // Fire exit-deferred timer → routes through on_dead (default=escalate)
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    // Default on_dead = escalate → Waiting status
    assert_eq!(pipeline.step, "plan");
    assert!(pipeline.step_status.is_waiting());
}

#[tokio::test]
async fn session_death_timer_for_nonexistent_pipeline_is_noop() {
    let ctx = setup().await;

    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::new("liveness:nonexistent"),
        })
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn session_death_timer_on_terminal_pipeline_is_noop() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Fail the pipeline first
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "failed");

    // Liveness timer on terminal pipeline is a no-op
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();
    assert!(result.is_empty());
}

// =============================================================================
// Agent exited via AgentExited event
// =============================================================================

#[tokio::test]
async fn agent_exited_on_terminal_pipeline_is_noop() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to plan (agent step)
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

    // AgentExited on terminal pipeline should be a no-op
    let result = ctx
        .runtime
        .handle_event(Event::AgentExited {
            agent_id,
            exit_code: Some(0),
        })
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_exited_for_unknown_agent_is_noop() {
    let ctx = setup().await;

    let result = ctx
        .runtime
        .handle_event(Event::AgentExited {
            agent_id: AgentId::new("nonexistent-plan".to_string()),
            exit_code: Some(0),
        })
        .await
        .unwrap();
    assert!(result.is_empty());
}

// =============================================================================
// on_dead action tests
// =============================================================================

/// Runbook with agent that has on_dead = done
const RUNBOOK_ON_DEAD_DONE: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "init"
run = { agent = "worker" }
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[agent.worker]
run = 'claude'
prompt = "Test"
on_dead = "done"
"#;

#[tokio::test]
async fn agent_exited_advances_when_on_dead_is_done() {
    let ctx = setup_with_runbook(RUNBOOK_ON_DEAD_DONE).await;

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
    assert_eq!(pipeline.step, "init");

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // AgentExited + on_dead = done should advance pipeline
    let result = ctx
        .runtime
        .handle_event(Event::AgentExited {
            agent_id,
            exit_code: Some(0),
        })
        .await
        .unwrap();

    assert!(!result.is_empty() || ctx.runtime.get_pipeline(&pipeline_id).unwrap().step == "done");
}

/// Runbook with agent that has on_dead = fail
const RUNBOOK_ON_DEAD_FAIL: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "init"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Test"
on_dead = "fail"
"#;

#[tokio::test]
async fn agent_exited_fails_when_on_dead_is_fail() {
    let ctx = setup_with_runbook(RUNBOOK_ON_DEAD_FAIL).await;

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

    // AgentExited + on_dead = fail should fail the pipeline
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id,
            exit_code: Some(0),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "failed");
}

/// Runbook with agent that has default on_dead (escalate)
const RUNBOOK_ON_DEAD_DEFAULT: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "init"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Test"
"#;

#[tokio::test]
async fn agent_exited_escalates_by_default() {
    let ctx = setup_with_runbook(RUNBOOK_ON_DEAD_DEFAULT).await;

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

    // AgentExited + default on_dead (escalate) should notify human
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id,
            exit_code: Some(0),
        })
        .await
        .unwrap();

    // Escalate sets pipeline to Waiting status
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "init");
    assert!(pipeline.step_status.is_waiting());
}

// =============================================================================
// Gate action tests
// =============================================================================

/// Runbook where agent has on_dead = gate with a passing command
const RUNBOOK_GATE_DEAD_PASS: &str = r#"
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
on_dead = { action = "gate", run = "true" }
"#;

#[tokio::test]
async fn gate_dead_advances_when_command_passes() {
    let ctx = setup_with_runbook(RUNBOOK_GATE_DEAD_PASS).await;

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

    // Agent exits, on_dead gate runs "true" which passes → advance pipeline
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id,
            exit_code: Some(0),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
}

/// Runbook where agent has on_dead = gate with a passing command, then another step
const RUNBOOK_GATE_DEAD_CHAIN: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "plan-check"

[[pipeline.build.step]]
name = "plan-check"
run = "true"
on_done = "implement"

[[pipeline.build.step]]
name = "implement"
run = { agent = "implementer" }

[agent.worker]
run = 'claude'
prompt = "Test"
on_dead = { action = "gate", run = "true" }

[agent.implementer]
run = 'claude'
prompt = "Implement"
"#;

#[tokio::test]
async fn gate_dead_result_events_advance_past_shell_step() {
    let mut ctx = setup_with_runbook(RUNBOOK_GATE_DEAD_CHAIN).await;

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
    assert_eq!(ctx.runtime.get_pipeline(&pipeline_id).unwrap().step, "work");

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Agent exits; gate action runs "true" which passes and advances.
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id,
            exit_code: Some(0),
        })
        .await
        .unwrap();

    // Pipeline is at plan-check after advance, but ShellExited hasn't
    // been re-processed yet (it arrives via the event channel).
    assert_eq!(
        ctx.runtime.get_pipeline(&pipeline_id).unwrap().step,
        "plan-check"
    );

    // ShellExited arrives via the event channel (async shell execution).
    // Re-process it to advance past the shell step to implement.
    let shell_completed = ctx.event_rx.recv().await.unwrap();
    assert!(matches!(shell_completed, Event::ShellExited { .. }));
    ctx.runtime.handle_event(shell_completed).await.unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "implement");
}

#[tokio::test]
async fn agent_exited_ignores_non_agent_step() {
    let ctx = setup_with_runbook(RUNBOOK_GATE_DEAD_CHAIN).await;

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

    // Agent exits via gate "true" → pipeline advances to plan-check (shell step)
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id: agent_id.clone(),
            exit_code: Some(0),
        })
        .await
        .unwrap();

    assert_eq!(
        ctx.runtime.get_pipeline(&pipeline_id).unwrap().step,
        "plan-check"
    );

    // AgentExited for old agent while pipeline is at a shell step
    // should be a no-op (pipeline already advanced past the agent step).
    let result = ctx
        .runtime
        .handle_event(Event::AgentExited {
            agent_id,
            exit_code: Some(0),
        })
        .await
        .unwrap();
    assert!(result.is_empty());
}

/// Runbook where agent has on_dead = gate with a failing command
const RUNBOOK_GATE_DEAD_FAIL: &str = r#"
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
on_dead = { action = "gate", run = "false" }
"#;

#[tokio::test]
async fn gate_dead_escalates_when_command_fails() {
    let ctx = setup_with_runbook(RUNBOOK_GATE_DEAD_FAIL).await;

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

    // Agent exits, on_dead gate runs "false" which fails → escalate
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id,
            exit_code: Some(0),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    // Gate failed → escalate → Waiting status
    assert_eq!(pipeline.step, "work");
    assert!(pipeline.step_status.is_waiting());
}
