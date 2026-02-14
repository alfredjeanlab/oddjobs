// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for agent lifecycle handlers (stop hooks, state change guards).

use crate::engine::test_helpers::{agent_exited, load_runbook_hash, setup_with_runbook};
use oj_core::{AgentId, DecisionId, DecisionOption, DecisionSource, Event, JobId, OwnerId};
use std::collections::HashMap;

const RESUME_ON_DEAD_RUNBOOK: &str = r#"
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
prompt = "Do work"
on_dead = { action = "resume", attempts = 3 }
"#;

/// Helper: apply job + step events and register agent, then create a
/// pending decision with the given source.
fn setup_waiting_job_with_decision(
    ctx: &crate::engine::test_helpers::TestContext,
    hash: &str,
    source: DecisionSource,
) {
    let events: Vec<Event> = vec![
        Event::JobCreated {
            id: JobId::from_string("job-1"),
            kind: "build".to_string(),
            name: "test".to_string(),
            runbook_hash: hash.to_string(),
            cwd: ctx.project_path.clone(),
            vars: HashMap::from([("name".to_string(), "feat".to_string())]),
            initial_step: "work".to_string(),
            created_at_ms: 1_000_000,
            project: "test".to_string(),
            cron: None,
        },
        Event::StepStarted {
            job_id: JobId::from_string("job-1"),
            step: "work".to_string(),
            agent_id: Some(AgentId::from_string("agent-1")),
            agent_name: Some("worker".to_string()),
        },
        Event::DecisionCreated {
            id: DecisionId::from_string("dec-1"),
            owner: OwnerId::Job(JobId::from_string("job-1")),
            project: "test".to_string(),
            created_at_ms: 2_000_000,
            agent_id: AgentId::from_string("agent-1"),
            source,
            context: "test decision".to_string(),
            options: vec![DecisionOption::new("Option A")],
            questions: None,
        },
    ];
    ctx.runtime.lock_state_mut(|state| {
        for event in &events {
            state.apply_event(event);
        }
    });
    ctx.runtime.register_agent(AgentId::from_string("agent-1"), JobId::from_string("job-1").into());
}

#[tokio::test]
async fn pending_dead_decision_suppresses_on_dead() {
    let ctx = setup_with_runbook(RESUME_ON_DEAD_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, RESUME_ON_DEAD_RUNBOOK);
    setup_waiting_job_with_decision(&ctx, &hash, DecisionSource::Dead);

    // AgentExited while a Dead decision is pending → suppressed (don't double-fire)
    let result = ctx
        .runtime
        .handle_event(agent_exited(
            AgentId::from_string("agent-1"),
            Some(0),
            JobId::from_string("job-1").into(),
        ))
        .await
        .unwrap();
    assert!(
        result.is_empty(),
        "on_dead should not fire when a Dead decision is pending, got: {:?}",
        result
    );
}

#[tokio::test]
async fn pending_alive_decision_auto_dismissed_on_agent_exit() {
    let ctx = setup_with_runbook(RESUME_ON_DEAD_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, RESUME_ON_DEAD_RUNBOOK);
    setup_waiting_job_with_decision(&ctx, &hash, DecisionSource::Idle);

    // AgentExited while an Idle (alive) decision is pending →
    // auto-dismiss the stale decision and fire on_dead
    let result = ctx
        .runtime
        .handle_event(agent_exited(
            AgentId::from_string("agent-1"),
            Some(1),
            JobId::from_string("job-1").into(),
        ))
        .await
        .unwrap();

    // The stale Idle decision should be resolved
    let decision = ctx.runtime.lock_state(|s| s.decisions.get("dec-1").cloned().unwrap());
    assert!(decision.is_resolved(), "stale Idle decision should be auto-dismissed");
    assert_eq!(decision.message.as_deref(), Some("auto-dismissed: agent exited"));

    // on_dead = resume should have fired (StepStarted for the respawned agent)
    let has_step_started = result.iter().any(|e| matches!(e, Event::StepStarted { .. }));
    assert!(
        has_step_started,
        "on_dead should fire after auto-dismissing stale alive decision, got: {:?}",
        result
    );
}

#[tokio::test]
async fn agent_exited_fires_on_dead_when_job_running() {
    let ctx = setup_with_runbook(RESUME_ON_DEAD_RUNBOOK).await;
    let hash = load_runbook_hash(&ctx, RESUME_ON_DEAD_RUNBOOK);
    let events = vec![
        Event::JobCreated {
            id: JobId::from_string("job-1"),
            kind: "build".to_string(),
            name: "test".to_string(),
            runbook_hash: hash,
            cwd: ctx.project_path.clone(),
            vars: HashMap::from([("name".to_string(), "feat".to_string())]),
            initial_step: "work".to_string(),
            created_at_ms: 1_000_000,
            project: "test".to_string(),
            cron: None,
        },
        Event::StepStarted {
            job_id: JobId::from_string("job-1"),
            step: "work".to_string(),
            agent_id: Some(AgentId::from_string("agent-1")),
            agent_name: Some("worker".to_string()),
        },
    ];
    ctx.runtime.lock_state_mut(|state| {
        for event in &events {
            state.apply_event(event);
        }
    });
    ctx.runtime.register_agent(AgentId::from_string("agent-1"), JobId::from_string("job-1").into());
    let result = ctx
        .runtime
        .handle_event(agent_exited(
            AgentId::from_string("agent-1"),
            Some(1),
            JobId::from_string("job-1").into(),
        ))
        .await
        .unwrap();
    let has_step_started = result.iter().any(|e| matches!(e, Event::StepStarted { .. }));
    assert!(has_step_started, "on_dead=resume should fire when job is running, got: {:?}", result);
}
