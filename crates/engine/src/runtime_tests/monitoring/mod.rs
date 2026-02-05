// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent monitoring timer and event handler tests

mod agent_state;
mod auto_resume;
mod dedup;
mod grace_timer;
mod session_cleanup;
mod timers;

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
