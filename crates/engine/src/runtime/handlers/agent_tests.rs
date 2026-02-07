// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for handle_agent_stop_hook (on_stop=escalate).

use crate::test_helpers::{load_runbook_hash, setup_with_runbook, TestContext};
use oj_core::{AgentId, AgentRunId, AgentRunStatus, Event, JobId, OwnerId};
use std::collections::HashMap;

// =============================================================================
// Runbook definitions
// =============================================================================

/// Runbook with a standalone agent command (on_stop triggers escalation)
const STANDALONE_RUNBOOK: &str = r#"
[command.agent_cmd]
args = "<name>"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the work"
on_idle = "done"
"#;

/// Runbook with job-based agent step
const JOB_RUNBOOK: &str = r#"
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
run = "claude"
prompt = "Do work"
"#;

// =============================================================================
// Helpers
// =============================================================================

/// Create a standalone agent run in Running status and register it.
fn create_running_agent_run(ctx: &TestContext, run_id: &str, agent_id_str: &str) {
    let agent_run_id = AgentRunId::new(run_id);
    let agent_id = AgentId::new(agent_id_str);

    let events = vec![
        Event::AgentRunCreated {
            id: agent_run_id.clone(),
            agent_name: "worker".to_string(),
            command_name: "agent_cmd".to_string(),
            namespace: "test".to_string(),
            cwd: ctx.project_root.clone(),
            runbook_hash: String::new(),
            vars: HashMap::new(),
            created_at_epoch_ms: 1_000_000,
        },
        Event::AgentRunStarted {
            id: agent_run_id,
            agent_id: agent_id.clone(),
        },
    ];
    ctx.runtime.lock_state_mut(|state| {
        for event in &events {
            state.apply_event(event);
        }
    });

    // Register agent → owner mapping (normally done by the runtime on spawn)
    ctx.runtime
        .register_agent(agent_id, OwnerId::agent_run(AgentRunId::new(run_id)));
}

/// Create a job on an agent step and register the agent.
async fn create_running_job(ctx: &TestContext, job_id: &str, agent_id_str: &str) {
    let hash = load_runbook_hash(ctx, JOB_RUNBOOK);
    let events = vec![
        Event::JobCreated {
            id: JobId::new(job_id),
            kind: "build".to_string(),
            name: "test-build".to_string(),
            runbook_hash: hash,
            cwd: ctx.project_root.clone(),
            vars: HashMap::from([("name".to_string(), "feat".to_string())]),
            initial_step: "work".to_string(),
            created_at_epoch_ms: 1_000_000,
            namespace: "test".to_string(),
            cron_name: None,
        },
        Event::StepStarted {
            job_id: JobId::new(job_id),
            step: "work".to_string(),
            agent_id: Some(AgentId::new(agent_id_str)),
            agent_name: Some("worker".to_string()),
        },
    ];
    ctx.runtime.lock_state_mut(|state| {
        for event in &events {
            state.apply_event(event);
        }
    });

    // Register agent → job mapping
    ctx.runtime
        .register_agent(AgentId::new(agent_id_str), OwnerId::Job(JobId::new(job_id)));
}

// =============================================================================
// Standalone agent: stop hook creates decision
// =============================================================================

#[tokio::test]
async fn stop_hook_standalone_creates_decision() {
    let ctx = setup_with_runbook(STANDALONE_RUNBOOK).await;
    create_running_agent_run(&ctx, "run-1", "agent-1");

    let events = ctx
        .runtime
        .handle_event(Event::AgentStop {
            agent_id: AgentId::new("agent-1"),
        })
        .await
        .unwrap();

    let decision = events
        .iter()
        .find(|e| matches!(e, Event::DecisionCreated { .. }));
    assert!(
        decision.is_some(),
        "expected DecisionCreated event, got: {:?}",
        events
    );

    // Verify the decision is for the right agent run
    if let Some(Event::DecisionCreated { owner, .. }) = decision {
        assert_eq!(*owner, OwnerId::agent_run(AgentRunId::new("run-1")));
    }
}

#[tokio::test]
async fn stop_hook_standalone_sets_escalated_status() {
    let ctx = setup_with_runbook(STANDALONE_RUNBOOK).await;
    create_running_agent_run(&ctx, "run-1", "agent-1");

    let events = ctx
        .runtime
        .handle_event(Event::AgentStop {
            agent_id: AgentId::new("agent-1"),
        })
        .await
        .unwrap();

    let status_event = events.iter().find(|e| {
        matches!(
            e,
            Event::AgentRunStatusChanged {
                status: AgentRunStatus::Escalated,
                ..
            }
        )
    });
    assert!(
        status_event.is_some(),
        "expected AgentRunStatusChanged to Escalated, got: {:?}",
        events
    );
}

#[tokio::test]
async fn stop_hook_standalone_idempotent_when_already_escalated() {
    let ctx = setup_with_runbook(STANDALONE_RUNBOOK).await;
    create_running_agent_run(&ctx, "run-1", "agent-1");

    // Set to Escalated
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::AgentRunStatusChanged {
            id: AgentRunId::new("run-1"),
            status: AgentRunStatus::Escalated,
            reason: Some("already".to_string()),
        });
    });

    let events = ctx
        .runtime
        .handle_event(Event::AgentStop {
            agent_id: AgentId::new("agent-1"),
        })
        .await
        .unwrap();

    assert!(
        events.is_empty(),
        "expected no-op for already-escalated agent run"
    );
}

#[tokio::test]
async fn stop_hook_standalone_unknown_agent_is_noop() {
    let ctx = setup_with_runbook(STANDALONE_RUNBOOK).await;

    let events = ctx
        .runtime
        .handle_event(Event::AgentStop {
            agent_id: AgentId::new("nonexistent"),
        })
        .await
        .unwrap();

    assert!(events.is_empty(), "expected no-op for unknown agent");
}

// =============================================================================
// Job agent: stop hook creates decision
// =============================================================================

#[tokio::test]
async fn stop_hook_job_creates_decision() {
    let ctx = setup_with_runbook(JOB_RUNBOOK).await;
    create_running_job(&ctx, "job-1", "agent-1").await;

    let events = ctx
        .runtime
        .handle_event(Event::AgentStop {
            agent_id: AgentId::new("agent-1"),
        })
        .await
        .unwrap();

    let decision = events
        .iter()
        .find(|e| matches!(e, Event::DecisionCreated { .. }));
    assert!(
        decision.is_some(),
        "expected DecisionCreated event for job, got: {:?}",
        events
    );

    if let Some(Event::DecisionCreated { owner, .. }) = decision {
        assert_eq!(*owner, OwnerId::Job(JobId::new("job-1")));
    }
}

#[tokio::test]
async fn stop_hook_job_sets_step_waiting_with_decision_id() {
    let ctx = setup_with_runbook(JOB_RUNBOOK).await;
    create_running_job(&ctx, "job-1", "agent-1").await;

    let events = ctx
        .runtime
        .handle_event(Event::AgentStop {
            agent_id: AgentId::new("agent-1"),
        })
        .await
        .unwrap();

    let waiting = events
        .iter()
        .find(|e| matches!(e, Event::StepWaiting { .. }));
    assert!(
        waiting.is_some(),
        "expected StepWaiting event, got: {:?}",
        events
    );

    if let Some(Event::StepWaiting { decision_id, .. }) = waiting {
        assert!(
            decision_id.is_some(),
            "StepWaiting should have a decision_id linking to the created decision"
        );
    }
}

#[tokio::test]
async fn stop_hook_job_idempotent_when_already_waiting() {
    let ctx = setup_with_runbook(JOB_RUNBOOK).await;
    create_running_job(&ctx, "job-1", "agent-1").await;

    // Set job step to waiting
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::StepWaiting {
            job_id: JobId::new("job-1"),
            step: "work".to_string(),
            reason: Some("already waiting".to_string()),
            decision_id: Some("dec-1".to_string()),
        });
    });

    let events = ctx
        .runtime
        .handle_event(Event::AgentStop {
            agent_id: AgentId::new("agent-1"),
        })
        .await
        .unwrap();

    assert!(
        events.is_empty(),
        "expected no-op for already-waiting job step"
    );
}
