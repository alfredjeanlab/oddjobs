// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

mod agent;
mod core;
mod shell;
mod worker;
mod workspace;

use super::*;
use crate::adapters::{
    workspace_adapter, AgentAdapterError, AgentReconnectConfig, FakeAgentAdapter, FakeNotifyAdapter,
};
use crate::engine::RuntimeDeps;
use oj_core::{AgentId, CrewId, FakeClock, JobId, OwnerId, TimerId, WorkspaceId};
use std::collections::HashMap;
use tokio::sync::mpsc;

type TestExecutor = Executor<FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

struct TestHarness {
    executor: TestExecutor,
    event_rx: mpsc::Receiver<Event>,
    agents: FakeAgentAdapter,
    notifier: FakeNotifyAdapter,
}

async fn setup() -> TestHarness {
    let (event_tx, event_rx) = mpsc::channel(100);
    let agents = FakeAgentAdapter::new();
    let notifier = FakeNotifyAdapter::new();

    let executor = Executor::new(
        RuntimeDeps {
            agents: agents.clone(),
            notifier: notifier.clone(),
            state: Arc::new(Mutex::new(MaterializedState::default())),
            workspace: workspace_adapter(false),
        },
        Arc::new(Mutex::new(Scheduler::new())),
        FakeClock::new(),
        event_tx,
    );

    TestHarness { executor, event_rx, agents, notifier }
}

/// Build a default `Effect::SpawnAgent` for executor tests.
fn spawn_agent(id: &str) -> Effect {
    Effect::SpawnAgent {
        agent_id: AgentId::new(id),
        agent_name: "builder".to_string(),
        owner: JobId::new("job-1").into(),
        workspace_path: std::path::PathBuf::from("/tmp/ws"),
        input: HashMap::new(),
        command: "claude".to_string(),
        env: vec![],
        cwd: None,
        unset_env: vec![],
        resume: false,
        container: None,
    }
}

/// Build a default `Effect::Shell` for executor tests.
fn shell(command: &str) -> Effect {
    Effect::Shell {
        owner: Some(JobId::new("test").into()),
        step: "init".to_string(),
        command: command.to_string(),
        cwd: std::path::PathBuf::from("/tmp"),
        env: HashMap::new(),
        container: None,
    }
}
