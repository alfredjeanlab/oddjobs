// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared test helpers for the engine crate.

use crate::adapters::{workspace_adapter, FakeAgentAdapter, FakeNotifyAdapter};
use crate::engine::spawn::{build_spawn_effects, SpawnCtx};
use crate::engine::{Runtime, RuntimeConfig, RuntimeDeps, RuntimeError};
use crate::storage::MaterializedState;
use oj_core::{AgentId, Clock, Effect, Event, FakeClock, JobId, OwnerId};
use oj_runbook::AgentDef;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;
use tokio::sync::mpsc;

/// Convenience alias for the fully-typed test runtime.
pub(crate) type TestRuntime = Runtime<FakeAgentAdapter, FakeNotifyAdapter, FakeClock>;

/// Test context holding the runtime, adapters, and project path.
pub(crate) struct TestContext {
    pub runtime: TestRuntime,
    pub clock: FakeClock,
    pub project_path: PathBuf,
    pub event_rx: mpsc::Receiver<Event>,
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

    let agents = FakeAgentAdapter::new();
    let notifier = FakeNotifyAdapter::new();
    let clock = FakeClock::new();
    let (event_tx, event_rx) = mpsc::channel(100);
    let runtime = Runtime::new(
        RuntimeDeps {
            agents: agents.clone(),
            notifier: notifier.clone(),
            state: Arc::new(Mutex::new(MaterializedState::default())),
            workspace: workspace_adapter(false),
        },
        clock.clone(),
        RuntimeConfig { state_dir: dir_path.clone(), log_dir: dir_path.join("logs") },
        event_tx,
    );

    TestContext { runtime, clock, project_path: dir_path, event_rx, agents, notifier }
}

impl TestContext {
    /// Drain background events (like AgentSpawned from deferred SpawnAgent)
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

/// Convenience wrapper for `build_spawn_effects` with empty input and no resume.
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
        false,
    )
}

// ---- Event construction helpers ----

pub(crate) fn shell_exited(job_id: &str, step: &str, exit_code: i32) -> Event {
    Event::ShellExited {
        job_id: JobId::new(job_id),
        step: step.to_string(),
        exit_code,
        stdout: None,
        stderr: None,
    }
}

pub(crate) fn shell_ok(job_id: &str, step: &str) -> Event {
    shell_exited(job_id, step, 0)
}

pub(crate) fn shell_fail(job_id: &str, step: &str) -> Event {
    shell_exited(job_id, step, 1)
}

pub(crate) fn agent_exited(agent_id: AgentId, exit_code: Option<i32>, owner: OwnerId) -> Event {
    Event::AgentExited { id: agent_id, exit_code, owner }
}

pub(crate) fn agent_waiting(agent_id: AgentId, owner: OwnerId) -> Event {
    Event::AgentWaiting { id: agent_id, owner }
}

pub(crate) fn worker_started(
    name: &str,
    root: &Path,
    hash: &str,
    queue: &str,
    concurrency: u32,
    ns: &str,
) -> Event {
    Event::WorkerStarted {
        worker: name.to_string(),
        project_path: root.to_path_buf(),
        runbook_hash: hash.to_string(),
        queue: queue.to_string(),
        concurrency,
        project: ns.to_string(),
    }
}

/// Build a `HashMap<String, String>` from key-value pairs.
macro_rules! vars {
    ($($key:expr => $val:expr),* $(,)?) => {{
        let mut map = std::collections::HashMap::<String, String>::new();
        $(map.insert($key.to_string(), $val.to_string());)*
        map
    }};
}
pub(crate) use vars;

impl TestContext {
    /// Collect all pending timer IDs by advancing the clock and draining fired timers.
    pub(crate) fn pending_timer_ids(&self) -> Vec<String> {
        let scheduler = self.runtime.executor.scheduler();
        let mut sched = scheduler.lock();
        self.clock.advance(std::time::Duration::from_secs(7200));
        let fired = sched.fired_timers(self.clock.now());
        fired
            .into_iter()
            .filter_map(|e| match e {
                Event::TimerStart { id } => Some(id.as_str().to_string()),
                _ => None,
            })
            .collect()
    }
}

/// Assert that no timer with the given prefix exists.
pub(crate) fn assert_no_timer_with_prefix(timer_ids: &[String], prefix: &str) {
    let matching: Vec<&String> = timer_ids.iter().filter(|id| id.starts_with(prefix)).collect();
    assert!(
        matching.is_empty(),
        "expected no timers starting with '{}', found: {:?}",
        prefix,
        matching
    );
}
