// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runtime tests

mod agent_run;
mod cron;
mod directives;
mod errors;
mod idempotency;
mod job_create;
mod job_deleted;
mod monitoring;
mod notify;
mod on_dead;
mod resume;
mod sessions;
mod steps;
mod timer_cleanup;
mod worker;

use super::*;
use crate::{RuntimeConfig, RuntimeDeps};
use oj_adapters::{FakeAgentAdapter, FakeNotifyAdapter, FakeSessionAdapter};
use oj_core::{AgentId, FakeClock, JobId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use tokio::sync::mpsc;

type TestRuntime = Runtime<FakeSessionAdapter, FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

/// Test context holding the runtime and project path
struct TestContext {
    runtime: TestRuntime,
    clock: FakeClock,
    project_root: PathBuf,
    event_rx: mpsc::Receiver<Event>,
    sessions: FakeSessionAdapter,
    agents: FakeAgentAdapter,
    notifier: FakeNotifyAdapter,
}

fn command_event(
    job_id: &str,
    job_name: &str,
    command: &str,
    args: HashMap<String, String>,
    project_root: &Path,
) -> Event {
    Event::CommandRun {
        job_id: JobId::new(job_id),
        job_name: job_name.to_string(),
        project_root: project_root.to_path_buf(),
        invoke_dir: project_root.to_path_buf(),
        command: command.to_string(),
        namespace: String::new(),
        args,
    }
}

const TEST_RUNBOOK: &str = r#"
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
on_done = "execute"

[[job.build.step]]
name = "execute"
run = { agent = "executor" }
on_done = "merge"

[[job.build.step]]
name = "merge"
run = "echo merge"
on_done = "done"
on_fail = "cleanup"

[[job.build.step]]
name = "done"
run = "echo done"

[[job.build.step]]
name = "cleanup"
run = "echo cleanup"

[agent.planner]
run = "claude --print"
[agent.planner.env]
OJ_STEP = "plan"

[agent.executor]
run = "claude --print"
[agent.executor.env]
OJ_STEP = "execute"
"#;

async fn setup() -> TestContext {
    setup_with_runbook(TEST_RUNBOOK).await
}

async fn setup_with_runbook(runbook_content: &str) -> TestContext {
    let dir = tempdir().unwrap();
    // Keep the temp directory alive by leaking it
    let dir_path = dir.keep();

    // Create project structure with runbook file
    let runbook_dir = dir_path.join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(runbook_dir.join("test.toml"), runbook_content).unwrap();

    let sessions = FakeSessionAdapter::new();
    let agents = FakeAgentAdapter::new();
    let notifier = FakeNotifyAdapter::new();
    let clock = FakeClock::new();
    let (event_tx, event_rx) = mpsc::channel(100);
    let runtime = Runtime::new(
        RuntimeDeps {
            sessions: sessions.clone(),
            agents: agents.clone(),
            notifier: notifier.clone(),
            state: Arc::new(Mutex::new(MaterializedState::default())),
        },
        clock.clone(),
        RuntimeConfig {
            state_dir: dir_path.clone(),
            log_dir: dir_path.join("logs"),
        },
        event_tx,
    );

    TestContext {
        runtime,
        clock,
        project_root: dir_path,
        event_rx,
        sessions,
        agents,
        notifier,
    }
}

async fn create_job(ctx: &TestContext) -> String {
    create_job_with_id(ctx, "pipe-1").await
}

/// Get the agent_id for a job's current step from step history.
fn get_agent_id(ctx: &TestContext, job_id: &str) -> Option<AgentId> {
    let job = ctx.runtime.get_job(job_id)?;
    job.step_history
        .iter()
        .rfind(|r| r.name == job.step)
        .and_then(|r| r.agent_id.clone())
        .map(AgentId::new)
}

async fn create_job_with_id(ctx: &TestContext, job_id: &str) -> String {
    let args: HashMap<String, String> = [
        ("name".to_string(), "test-feature".to_string()),
        ("prompt".to_string(), "Add login".to_string()),
    ]
    .into_iter()
    .collect();

    ctx.runtime
        .handle_event(command_event(
            job_id,
            "build",
            "build",
            args,
            &ctx.project_root,
        ))
        .await
        .unwrap();

    job_id.to_string()
}

#[tokio::test]
async fn runtime_handle_command() {
    let ctx = setup().await;
    let _job_id = create_job(&ctx).await;

    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1);

    let job = jobs.values().next().unwrap();
    assert_eq!(job.name, "test-feature");
    assert_eq!(job.kind, "build");
}

#[tokio::test]
async fn shell_completion_advances_step() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Job starts at init step (shell)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "init");

    // Simulate shell completion
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
}

#[tokio::test]
async fn agent_done_advances_step() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to plan step (agent)
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

    // Advance job (orchestrator-driven)
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "execute");
}
