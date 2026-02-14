// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent exit behavior tests

use super::*;
use oj_core::{JobId, OwnerId, TimerId};

fn runbook_on_dead(on_dead: &str) -> String {
    format!(
        "[command.build]\nargs = \"<name>\"\nrun = {{ job = \"build\" }}\n\n\
         [job.build]\ninput = [\"name\"]\n\n\
         [[job.build.step]]\nname = \"work\"\nrun = {{ agent = \"worker\" }}\non_done = {{ step = \"done\" }}\n\n\
         [[job.build.step]]\nname = \"done\"\nrun = \"echo done\"\n\n\
         [agent.worker]\nrun = 'claude'\nprompt = \"Test\"\n{on_dead}\n"
    )
}

async fn setup_and_fire_on_dead(on_dead: &str) -> (TestContext, String) {
    let runbook = runbook_on_dead(on_dead);
    let mut ctx = setup_with_runbook(&runbook).await;
    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    ctx.runtime
        .handle_event(agent_exited(agent_id, Some(0), JobId::from_string(&job_id).into()))
        .await
        .unwrap();
    (ctx, job_id)
}

#[tokio::test]
async fn agent_death_triggers_on_dead_action() {
    let mut ctx = setup().await;
    let job_id = create_job(&ctx).await;
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();
    ctx.process_background_events().await;
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    ctx.agents.set_agent_alive(&agent_id, false);
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::from_string(job_id.clone())),
        })
        .await
        .unwrap();
    ctx.agents.set_agent_state(&agent_id, oj_core::AgentState::Exited { exit_code: Some(0) });
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: TimerId::exit_deferred(&JobId::from_string(job_id.clone())),
        })
        .await
        .unwrap();
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");
    assert!(job.step_status.is_waiting());
}

#[tokio::test]
async fn session_death_timer_for_nonexistent_job_is_noop() {
    let ctx = setup().await;
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart { id: TimerId::from_string("liveness:nonexistent") })
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn session_death_timer_on_terminal_job_is_noop() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;
    ctx.runtime.handle_event(shell_fail(&job_id, "init")).await.unwrap();
    let result = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: TimerId::liveness(&JobId::from_string(job_id.clone())),
        })
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_exited_on_terminal_job_is_noop() {
    let mut ctx = setup().await;
    let job_id = create_job(&ctx).await;
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();
    ctx.process_background_events().await;
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    ctx.runtime.handle_event(shell_fail(&job_id, "plan")).await.unwrap();
    assert!(ctx.runtime.get_job(&job_id).unwrap().is_terminal());
    let result = ctx
        .runtime
        .handle_event(agent_exited(agent_id, Some(0), JobId::from_string(&job_id).into()))
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_exited_for_unknown_agent_is_noop() {
    let ctx = setup().await;
    let result = ctx
        .runtime
        .handle_event(agent_exited(
            AgentId::from_string("nonexistent-plan"),
            Some(0),
            OwnerId::Job(JobId::default()),
        ))
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn agent_exited_advances_when_on_dead_is_done() {
    let (ctx, job_id) = setup_and_fire_on_dead("on_dead = \"done\"").await;
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "done");
}

#[tokio::test]
async fn agent_exited_fails_when_on_dead_is_fail() {
    let (ctx, job_id) = setup_and_fire_on_dead("on_dead = \"fail\"").await;
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "failed");
}

#[tokio::test]
async fn agent_exited_escalates_by_default() {
    let (ctx, job_id) = setup_and_fire_on_dead("").await;
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(job.step_status.is_waiting());
}

#[tokio::test]
async fn gate_dead_advances_when_command_passes() {
    let (ctx, job_id) =
        setup_and_fire_on_dead(r#"on_dead = { action = "gate", run = "true" }"#).await;
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "done");
}

fn gate_dead_chain_runbook() -> String {
    let mut runbook = test_runbook_steps(
        "build",
        "",
        &[
            ("work", "{ agent = \"worker\" }", "on_done = { step = \"plan-check\" }"),
            ("plan-check", "true", "on_done = { step = \"implement\" }"),
            ("implement", "{ agent = \"implementer\" }", ""),
        ],
    );
    runbook.push_str("\n[agent.worker]\nrun = 'claude'\nprompt = \"Test\"\non_dead = { action = \"gate\", run = \"true\" }\n\n[agent.implementer]\nrun = 'claude'\nprompt = \"Implement\"\n");
    runbook
}

#[tokio::test]
async fn gate_dead_result_events_advance_past_shell_step() {
    let mut ctx = setup_with_runbook(&gate_dead_chain_runbook()).await;
    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "work");
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    ctx.runtime
        .handle_event(agent_exited(agent_id, Some(0), JobId::from_string(&job_id).into()))
        .await
        .unwrap();
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "plan-check");
    let shell_completed = ctx.event_rx.recv().await.unwrap();
    assert!(matches!(shell_completed, Event::ShellExited { .. }));
    ctx.runtime.handle_event(shell_completed).await.unwrap();
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "implement");
}

#[tokio::test]
async fn agent_exited_ignores_non_agent_step() {
    let mut ctx = setup_with_runbook(&gate_dead_chain_runbook()).await;
    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    ctx.process_background_events().await;
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();
    ctx.runtime
        .handle_event(agent_exited(agent_id.clone(), Some(0), JobId::from_string(&job_id).into()))
        .await
        .unwrap();
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "plan-check");
    let result = ctx
        .runtime
        .handle_event(agent_exited(agent_id, Some(0), JobId::from_string(&job_id).into()))
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn gate_dead_escalates_when_command_fails() {
    let (ctx, job_id) =
        setup_and_fire_on_dead(r#"on_dead = { action = "gate", run = "false" }"#).await;
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(job.step_status.is_waiting());
}
