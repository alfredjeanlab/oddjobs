// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for lifecycle guard logic in handle_monitor_state_for:
//! auto-dismissing stale alive decisions on agent death, blocking on
//! pending dead decisions, etc.

use super::*;

/// Runbook with on_idle=escalate (creates alive decision) and on_dead=done.
const ESCALATE_IDLE_DONE_DEAD: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[[job.build.step]]
name = "work"
run = { agent = "worker" }
on_done = { step = "done" }

[[job.build.step]]
name = "done"
run = "echo done"

[agent.worker]
run = "claude"
prompt = "Do the work"
on_idle = "escalate"
on_dead = "done"
"#;

#[tokio::test]
async fn dead_event_auto_dismisses_pending_idle_decision() {
    let mut ctx = setup_with_runbook(ESCALATE_IDLE_DONE_DEAD).await;
    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Agent goes idle → on_idle = escalate → creates Idle decision, step Waiting
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(agent_waiting(agent_id.clone(), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting(), "step should be waiting after idle escalation");

    let decisions_before =
        ctx.runtime.lock_state(|s| s.decisions.values().filter(|d| !d.is_resolved()).count());
    assert_eq!(decisions_before, 1, "should have 1 pending decision");

    // Agent dies → should auto-dismiss the stale Idle decision and fire on_dead = done
    ctx.runtime
        .handle_event(agent_exited(agent_id.clone(), Some(1), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    // The stale Idle decision should be resolved
    let pending_after =
        ctx.runtime.lock_state(|s| s.decisions.values().filter(|d| !d.is_resolved()).count());
    assert_eq!(pending_after, 0, "stale Idle decision should be auto-dismissed");

    let dismissed = ctx
        .runtime
        .lock_state(|s| s.decisions.values().find(|d| d.is_resolved()).map(|d| d.message.clone()));
    assert_eq!(dismissed.unwrap().as_deref(), Some("auto-dismissed: agent exited"),);

    // on_dead = done should have advanced the job
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done", "on_dead=done should advance job after auto-dismiss");
}

#[tokio::test]
async fn dead_event_blocked_by_pending_dead_decision() {
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Test\"\non_dead = \"escalate\"",
    ))
    .await;

    let mut ctx_mut = ctx;
    let job_id = create_job_for_runbook(&ctx_mut, "build", &[]).await;
    ctx_mut.process_background_events().await;

    let agent_id = get_agent_id(&ctx_mut, &job_id).unwrap();

    // Agent dies → on_dead = escalate → creates Dead decision
    ctx_mut
        .runtime
        .handle_event(agent_exited(agent_id.clone(), Some(1), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    let job = ctx_mut.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting(), "step should be waiting after dead escalation");

    let decisions =
        ctx_mut.runtime.lock_state(|s| s.decisions.values().filter(|d| !d.is_resolved()).count());
    assert_eq!(decisions, 1, "should have 1 pending Dead decision");

    // Second death event (e.g. AgentGone after AgentExited) → should be blocked
    let result = ctx_mut
        .runtime
        .handle_event(Event::AgentGone {
            id: agent_id.clone(),
            owner: JobId::from_string(&job_id).into(),
            exit_code: None,
        })
        .await
        .unwrap();

    assert!(result.is_empty(), "second dead event should be blocked by pending Dead decision");

    // Decision should still be pending (not double-fired)
    let still_pending =
        ctx_mut.runtime.lock_state(|s| s.decisions.values().filter(|d| !d.is_resolved()).count());
    assert_eq!(still_pending, 1, "Dead decision should still be pending");
}

#[tokio::test]
async fn idle_event_blocked_by_pending_idle_decision() {
    // This is already tested by duplicate_idle_creates_only_one_decision in dedup.rs,
    // but we verify it still works with the new source-aware guard.
    let ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Test\"\non_idle = \"escalate\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // First idle → escalate → Idle decision created
    ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());

    // Second idle → should be blocked by pending Idle decision
    let result =
        ctx.runtime.handle_event(Event::AgentStopBlocked { id: agent_id.clone() }).await.unwrap();

    assert!(result.is_empty(), "second idle should be blocked by pending Idle decision");
}

#[tokio::test]
async fn dead_event_proceeds_without_pending_decision() {
    // on_dead = done, no prior idle escalation → on_dead fires directly
    let mut ctx = setup_with_runbook(&test_runbook(
        "work",
        "done",
        "run = 'claude'\nprompt = \"Test\"\non_dead = \"done\"",
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;

    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // No pending decisions
    let pending =
        ctx.runtime.lock_state(|s| s.decisions.values().filter(|d| !d.is_resolved()).count());
    assert_eq!(pending, 0, "should have no pending decisions");

    // Agent dies → on_dead = done should fire normally
    ctx.runtime
        .handle_event(agent_exited(agent_id, Some(0), JobId::from_string(&job_id).into()))
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done", "on_dead=done should advance job when no decision pending");
}
