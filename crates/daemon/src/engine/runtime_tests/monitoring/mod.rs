// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent monitoring timer and event handler tests

mod agent_state;
mod auto_resume;
mod dedup;
mod lifecycle_guards;
mod session_cleanup;
mod timers;

use super::*;
use crate::adapters::AgentCall;
use oj_core::{CrewId, JobId, OwnerId, StepStatus, TimerId};

/// Helper: create a job and advance it to the "plan" agent step.
///
/// Returns (job_id, agent_id).
async fn setup_job_at_agent_step(ctx: &mut TestContext) -> (String, AgentId) {
    let job_id = create_job(ctx).await;

    // Advance past init (shell) to plan (agent)
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    // SpawnAgent is now deferred; drain background events to apply AgentSpawned
    ctx.process_background_events().await;

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");

    let agent_id = get_agent_id(ctx, &job_id).unwrap();

    (job_id, agent_id)
}

/// Helper: spawn a standalone agent and return (crew_id, agent_id)
async fn setup_standalone_agent(ctx: &mut TestContext) -> (String, AgentId) {
    handle_event_chain(
        ctx,
        crew_command_event(
            "job-1",
            "worker",
            "agent_cmd",
            vars!("name" => "test"),
            &ctx.project_path,
        ),
    )
    .await;

    // SpawnAgent is now deferred; drain background events to apply AgentSpawned
    ctx.process_background_events().await;

    let crew_id = "job-1".to_string();
    let crew = ctx.runtime.lock_state(|s| s.crew.get("job-1").cloned()).unwrap();
    let agent_id = AgentId::new(crew.agent_id.as_ref().unwrap());

    (crew_id, agent_id)
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
on_done = { step = "plan" }

[[job.build.step]]
name = "plan"
run = { agent = "planner" }
on_done = { step = "done" }

[[job.build.step]]
name = "done"
run = "echo done"

[agent.planner]
run = "claude --print"
on_idle = "done"
on_dead = "done"
on_error = "fail"
"#;
