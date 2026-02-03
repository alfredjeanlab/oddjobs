// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer lifecycle cleanup tests.
//!
//! Verifies that liveness, exit-deferred, and cooldown timers are properly
//! cleaned up when pipelines advance, complete, fail, or are cancelled.

use super::*;
use oj_core::{PipelineId, TimerId};

/// Collect all pending timer IDs from the scheduler by advancing time far
/// into the future and draining fired timers.
fn pending_timer_ids(ctx: &TestContext) -> Vec<String> {
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(std::time::Duration::from_secs(7200));
    let fired = sched.fired_timers(ctx.clock.now());
    fired
        .into_iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str().to_string()),
            _ => None,
        })
        .collect()
}

/// Helper: check that no pipeline-scoped timer with the given prefix exists.
fn assert_no_timer_with_prefix(timer_ids: &[String], prefix: &str) {
    let matching: Vec<&String> = timer_ids
        .iter()
        .filter(|id| id.starts_with(prefix))
        .collect();
    assert!(
        matching.is_empty(),
        "expected no timers starting with '{}', found: {:?}",
        prefix,
        matching
    );
}

// =============================================================================
// Runbook definitions
// =============================================================================

/// Runbook with on_idle = done, on_dead = done (clean advance path)
const RUNBOOK_CLEANUP: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "finish"

[[pipeline.build.step]]
name = "finish"
run = "echo done"

[agent.worker]
run = "claude --print"
on_idle = "done"
on_dead = "done"
"#;

/// Runbook with on_idle = nudge with attempts and cooldown
const RUNBOOK_COOLDOWN: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "finish"

[[pipeline.build.step]]
name = "finish"
run = "echo done"

[agent.worker]
run = "claude --print"
on_idle = { action = "nudge", attempts = 3, cooldown = "10s" }
on_dead = "done"
"#;

// =============================================================================
// Liveness timer cleanup
// =============================================================================

#[tokio::test]
async fn liveness_timer_cancelled_when_pipeline_advances_past_agent_step() {
    let ctx = setup_with_runbook(RUNBOOK_CLEANUP).await;

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

    // Liveness timer should be pending from spawn
    let scheduler = ctx.runtime.executor.scheduler();
    assert!(scheduler.lock().has_timers());

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Agent goes idle → on_idle = done → pipeline advances to "finish"
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "finish");

    // Liveness timer should be cancelled (not pending)
    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", pipeline_id));
}

#[tokio::test]
async fn liveness_timer_cancelled_on_pipeline_failure() {
    let ctx = setup_with_runbook(RUNBOOK_CLEANUP).await;

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

    // Simulate agent exiting with failure → ShellExited exit_code=1 on agent step
    // triggers fail_pipeline which cancels timers
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.is_terminal());

    // No liveness timers should remain
    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", pipeline_id));
}

#[tokio::test]
async fn liveness_timer_cancelled_on_pipeline_cancellation() {
    let ctx = setup_with_runbook(RUNBOOK_CLEANUP).await;

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

    // Cancel the pipeline
    ctx.runtime
        .handle_event(Event::PipelineCancel {
            id: PipelineId::new(pipeline_id.clone()),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.is_terminal());

    // No liveness timers should remain
    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", pipeline_id));
}

// =============================================================================
// Exit-deferred timer cleanup
// =============================================================================

#[tokio::test]
async fn exit_deferred_timer_cancelled_when_pipeline_advances() {
    let ctx = setup_with_runbook(RUNBOOK_CLEANUP).await;

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

    // Session not registered → liveness detects death → schedules exit-deferred
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // Verify exit-deferred was scheduled
    {
        let scheduler = ctx.runtime.executor.scheduler();
        assert!(scheduler.lock().has_timers());
    }

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // Agent goes idle → on_idle = done → pipeline advances (before exit-deferred fires)
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "finish");

    // Exit-deferred timer should have been cancelled during advance
    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", pipeline_id));
}

#[tokio::test]
async fn exit_deferred_timer_cancelled_on_pipeline_failure() {
    let ctx = setup_with_runbook(RUNBOOK_CLEANUP).await;

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

    // Session dead → liveness schedules exit-deferred
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // Fail the pipeline before exit-deferred fires
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.is_terminal());

    // Exit-deferred should be cancelled
    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", pipeline_id));
}

#[tokio::test]
async fn exit_deferred_timer_cancelled_on_pipeline_cancellation() {
    let ctx = setup_with_runbook(RUNBOOK_CLEANUP).await;

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

    // Session dead → liveness schedules exit-deferred
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // Cancel before exit-deferred fires
    ctx.runtime
        .handle_event(Event::PipelineCancel {
            id: PipelineId::new(pipeline_id.clone()),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.is_terminal());

    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", pipeline_id));
}

// =============================================================================
// Cooldown timer cleanup
// =============================================================================

#[tokio::test]
async fn cooldown_timer_noop_when_pipeline_becomes_terminal() {
    let ctx = setup_with_runbook(RUNBOOK_COOLDOWN).await;

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

    // Register session as alive so nudge (SendToSession) doesn't fail
    let session_id = pipeline.session_id.clone().unwrap();
    ctx.sessions.add_session(&session_id, true);

    let agent_id = get_agent_id(&ctx, &pipeline_id).unwrap();

    // First idle → on_idle nudge (attempt 1, no cooldown yet)
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Second idle → attempt 2 → cooldown timer scheduled
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    // Verify cooldown timer was scheduled
    {
        let scheduler = ctx.runtime.executor.scheduler();
        let sched = scheduler.lock();
        assert!(sched.has_timers(), "cooldown timer should be pending");
    }

    // Fail the pipeline while cooldown is pending
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert!(pipeline.is_terminal());

    // Fire the cooldown timer — should be a no-op since pipeline is terminal
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::cooldown(&PipelineId::new(pipeline_id.clone()), "idle", 0),
        })
        .await
        .unwrap();

    assert!(
        result.is_empty(),
        "cooldown on terminal pipeline should be a no-op"
    );
}

#[tokio::test]
async fn cooldown_timer_noop_when_pipeline_missing() {
    let ctx = setup_with_runbook(RUNBOOK_COOLDOWN).await;

    // Fire cooldown timer for a pipeline that doesn't exist
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::cooldown(&PipelineId::new("nonexistent"), "idle", 0),
        })
        .await
        .unwrap();

    assert!(result.is_empty());
}

// =============================================================================
// Combined cleanup: full lifecycle
// =============================================================================

#[tokio::test]
async fn all_pipeline_timers_cancelled_after_on_dead_done_completes() {
    let ctx = setup_with_runbook(RUNBOOK_CLEANUP).await;

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

    // Full lifecycle: liveness detects dead session → exit-deferred → on_dead=done
    // Step 1: Liveness fires, session dead → exit-deferred scheduled
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // Step 2: Set agent state to Exited for on_dead
    ctx.agents.set_agent_state(
        &agent_id,
        oj_core::AgentState::Exited { exit_code: Some(0) },
    );

    // Step 3: Exit-deferred fires → on_dead=done → pipeline advances to "finish"
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "finish");

    // No liveness or exit-deferred timers should remain for this pipeline
    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", pipeline_id));
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", pipeline_id));
}

#[tokio::test]
async fn all_pipeline_timers_cancelled_after_on_idle_done_completes() {
    let ctx = setup_with_runbook(RUNBOOK_CLEANUP).await;

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

    // Session dead → liveness → exit-deferred (both timers now exist)
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&PipelineId::new(pipeline_id.clone())),
        })
        .await
        .unwrap();

    // Agent goes idle → on_idle=done → pipeline advances to "finish"
    // This should cancel BOTH liveness and exit-deferred timers
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "finish");

    // Both timers should be gone
    let timer_ids = pending_timer_ids(&ctx);
    assert_no_timer_with_prefix(&timer_ids, &format!("liveness:{}", pipeline_id));
    assert_no_timer_with_prefix(&timer_ids, &format!("exit-deferred:{}", pipeline_id));
}
