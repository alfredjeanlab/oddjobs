// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

mod agent;
mod core;
mod shell;
mod worker;
mod workspace;

use super::*;
use crate::RuntimeDeps;
use oj_adapters::{
    AgentError, AgentReconnectConfig, FakeAgentAdapter, FakeNotifyAdapter, FakeSessionAdapter,
};
use oj_core::{AgentId, AgentRunId, FakeClock, JobId, OwnerId, SessionId, TimerId, WorkspaceId};
use std::collections::HashMap;
use tokio::sync::mpsc;

type TestExecutor = Executor<FakeSessionAdapter, FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

struct TestHarness {
    executor: TestExecutor,
    event_rx: mpsc::Receiver<Event>,
    sessions: FakeSessionAdapter,
    agents: FakeAgentAdapter,
    notifier: FakeNotifyAdapter,
}

async fn setup() -> TestHarness {
    let (event_tx, event_rx) = mpsc::channel(100);
    let sessions = FakeSessionAdapter::new();
    let agents = FakeAgentAdapter::new();
    let notifier = FakeNotifyAdapter::new();

    let executor = Executor::new(
        RuntimeDeps {
            sessions: sessions.clone(),
            agents: agents.clone(),
            notifier: notifier.clone(),
            state: Arc::new(Mutex::new(MaterializedState::default())),
        },
        Arc::new(Mutex::new(Scheduler::new())),
        FakeClock::new(),
        event_tx,
    );

    TestHarness {
        executor,
        event_rx,
        sessions,
        agents,
        notifier,
    }
}
