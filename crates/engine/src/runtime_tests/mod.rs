// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runtime tests

mod directives;
mod errors;
mod monitoring;
mod notify;
mod on_dead;
mod resume;
mod sessions;
mod steps;
mod worker;

use super::*;
use crate::{RuntimeConfig, RuntimeDeps};
use oj_adapters::{FakeAgentAdapter, FakeNotifyAdapter, FakeSessionAdapter};
use oj_core::{AgentId, FakeClock, PipelineId};
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
    pipeline_id: &str,
    pipeline_name: &str,
    command: &str,
    args: HashMap<String, String>,
    project_root: &Path,
) -> Event {
    Event::CommandRun {
        pipeline_id: PipelineId::new(pipeline_id),
        pipeline_name: pipeline_name.to_string(),
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
run = { pipeline = "build" }

[pipeline.build]
input  = ["name", "prompt"]

[[pipeline.build.step]]
name = "init"
run = "echo init"
on_done = "plan"

[[pipeline.build.step]]
name = "plan"
run = { agent = "planner" }
on_done = "execute"

[[pipeline.build.step]]
name = "execute"
run = { agent = "executor" }
on_done = "merge"

[[pipeline.build.step]]
name = "merge"
run = "echo merge"
on_done = "done"
on_fail = "cleanup"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[[pipeline.build.step]]
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

    // Create workspaces directory
    let workspaces = dir_path.join("workspaces");
    std::fs::create_dir_all(&workspaces).unwrap();

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
            workspaces_root: workspaces,
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

async fn create_pipeline(ctx: &TestContext) -> String {
    create_pipeline_with_id(ctx, "pipe-1").await
}

/// Get the agent_id for a pipeline's current step from step history.
fn get_agent_id(ctx: &TestContext, pipeline_id: &str) -> Option<AgentId> {
    let pipeline = ctx.runtime.get_pipeline(pipeline_id)?;
    pipeline
        .step_history
        .iter()
        .rfind(|r| r.name == pipeline.step)
        .and_then(|r| r.agent_id.clone())
        .map(AgentId::new)
}

async fn create_pipeline_with_id(ctx: &TestContext, pipeline_id: &str) -> String {
    let args: HashMap<String, String> = [
        ("name".to_string(), "test-feature".to_string()),
        ("prompt".to_string(), "Add login".to_string()),
    ]
    .into_iter()
    .collect();

    ctx.runtime
        .handle_event(command_event(
            pipeline_id,
            "build",
            "build",
            args,
            &ctx.project_root,
        ))
        .await
        .unwrap();

    pipeline_id.to_string()
}

#[tokio::test]
async fn runtime_handle_command() {
    let ctx = setup().await;
    let _pipeline_id = create_pipeline(&ctx).await;

    let pipelines = ctx.runtime.pipelines();
    assert_eq!(pipelines.len(), 1);

    let pipeline = pipelines.values().next().unwrap();
    assert_eq!(pipeline.name, "test-feature");
    assert_eq!(pipeline.kind, "build");
}

#[tokio::test]
async fn shell_completion_advances_step() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Pipeline starts at init step (shell)
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "init");

    // Simulate shell completion
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
}

#[tokio::test]
async fn agent_done_advances_step() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to plan step (agent)
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

    // Advance pipeline (orchestrator-driven)
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "execute");
}
