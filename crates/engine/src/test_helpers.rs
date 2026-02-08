// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared test helpers for the engine crate.

use crate::spawn::{build_spawn_effects, SpawnCtx};
use crate::{Runtime, RuntimeConfig, RuntimeDeps, RuntimeError};
use oj_adapters::{FakeAgentAdapter, FakeNotifyAdapter, FakeSessionAdapter};
use oj_core::{Effect, Event, FakeClock};
use oj_runbook::AgentDef;
use oj_storage::MaterializedState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;
use tokio::sync::mpsc;

/// Convenience alias for the fully-typed test runtime.
pub(crate) type TestRuntime =
    Runtime<FakeSessionAdapter, FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

/// Test context holding the runtime, adapters, and project path.
pub(crate) struct TestContext {
    pub runtime: TestRuntime,
    pub clock: FakeClock,
    pub project_root: PathBuf,
    pub event_rx: mpsc::Receiver<Event>,
    pub sessions: FakeSessionAdapter,
    pub agents: FakeAgentAdapter,
    pub notifier: FakeNotifyAdapter,
}

/// Create a test runtime with a runbook file on disk.
pub(crate) async fn setup_with_runbook(runbook_content: &str) -> TestContext {
    let dir = tempdir().unwrap();
    let dir_path = dir.keep();

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

impl TestContext {
    /// Drain background events (like SessionCreated from deferred SpawnAgent)
    /// and process them through state + handle_event.
    ///
    /// Call after operations that spawn agents (e.g. handle_event for CommandRun
    /// or ShellExited that advances to an agent step).
    pub(crate) async fn process_background_events(&mut self) {
        // Yield to let tokio::spawn tasks complete (FakeAgentAdapter is synchronous)
        tokio::task::yield_now().await;

        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }

        for event in events {
            self.runtime.lock_state_mut(|state| {
                state.apply_event(&event);
            });
            let _ = self.runtime.handle_event(event).await;
        }
    }
}

/// Parse a runbook, load it into cache + state, and return its hash.
pub(crate) fn load_runbook_hash(ctx: &TestContext, content: &str) -> String {
    let runbook = oj_runbook::parse_runbook(content).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };
    {
        let mut cache = ctx.runtime.runbook_cache.lock();
        cache.insert(hash.clone(), runbook);
    }
    ctx.runtime.lock_state_mut(|state| {
        state.apply_event(&Event::RunbookLoaded {
            hash: hash.clone(),
            version: 1,
            runbook: runbook_json,
        });
    });
    hash
}

/// Convenience wrapper for `build_spawn_effects` with empty input and no resume session.
pub(crate) fn spawn_effects(
    agent_def: &AgentDef,
    ctx: &SpawnCtx<'_>,
    agent_name: &str,
    workspace_path: &Path,
    state_dir: &Path,
) -> Result<Vec<Effect>, RuntimeError> {
    build_spawn_effects(
        agent_def,
        ctx,
        agent_name,
        &HashMap::new(),
        workspace_path,
        state_dir,
        None,
    )
}
