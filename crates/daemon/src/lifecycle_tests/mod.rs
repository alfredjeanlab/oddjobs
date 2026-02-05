// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

use crate::event_bus::{EventBus, EventReader};
use oj_adapters::{
    ClaudeAgentAdapter, DesktopNotifyAdapter, TmuxAdapter, TracedAgent, TracedSession,
};
use oj_core::{
    AgentRun, AgentRunId, AgentRunStatus, Event, Job, JobConfig, JobId, StepOutcome, StepRecord,
    StepStatus, SystemClock,
};
use oj_engine::{Runtime, RuntimeConfig, RuntimeDeps};
use oj_runbook::{JobDef, RunDirective, Runbook, StepDef};
use oj_storage::{load_snapshot, MaterializedState, Wal, WorkerRecord};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;
use tokio::sync::mpsc;

mod event_processing;
mod reconciliation;
mod startup_shutdown;

/// Build a minimal runbook with a single-step job.
fn test_runbook() -> Runbook {
    let mut jobs = HashMap::new();
    jobs.insert(
        "test".to_string(),
        JobDef {
            kind: "test".to_string(),
            name: None,
            vars: vec![],
            defaults: HashMap::new(),
            locals: HashMap::new(),
            cwd: None,
            workspace: None,
            on_done: None,
            on_fail: None,
            on_cancel: None,
            notify: Default::default(),
            steps: vec![StepDef {
                name: "only-step".to_string(),
                run: RunDirective::Shell("echo done".to_string()),
                on_done: None,
                on_fail: None,
                on_cancel: None,
            }],
        },
    );
    Runbook {
        commands: HashMap::new(),
        jobs,
        agents: HashMap::new(),
        queues: HashMap::new(),
        workers: HashMap::new(),
        crons: HashMap::new(),
    }
}

/// Hash a runbook the same way the runtime does.
fn runbook_hash(runbook: &Runbook) -> String {
    let json = serde_json::to_value(runbook).unwrap();
    let canonical = serde_json::to_string(&json).unwrap();
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{:x}", digest)
}

/// Set up a DaemonState with a job ready for step completion.
///
/// Returns the state and a WAL path for verification.
async fn setup_daemon_with_job() -> (DaemonState, PathBuf) {
    let (daemon, _, wal_path) = setup_daemon_with_job_and_reader().await;
    (daemon, wal_path)
}

/// Like `setup_daemon_with_job` but also returns the EventReader
/// so callers can simulate the main loop (mark_processed, etc.).
async fn setup_daemon_with_job_and_reader() -> (DaemonState, EventReader, PathBuf) {
    let dir = tempdir().unwrap();
    let dir_path = dir.keep();

    let wal_path = dir_path.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, event_reader) = EventBus::new(wal);

    // Build runbook and hash
    let runbook = test_runbook();
    let hash = runbook_hash(&runbook);
    let runbook_json = serde_json::to_value(&runbook).unwrap();

    // Pre-populate state with job + stored runbook
    let mut state = MaterializedState::default();
    let config = JobConfig {
        id: "pipe-1".to_string(),
        name: "test-job".to_string(),
        kind: "test".to_string(),
        vars: HashMap::new(),
        runbook_hash: hash.clone(),
        cwd: dir_path.clone(),
        initial_step: "only-step".to_string(),
        namespace: String::new(),
        cron_name: None,
    };
    let job = oj_core::Job::new(config, &SystemClock);
    state.jobs.insert("pipe-1".to_string(), job);
    state.apply_event(&Event::RunbookLoaded {
        hash,
        version: 1,
        runbook: runbook_json,
    });

    // Mark job step as running (as it would be during normal execution)
    state.jobs.get_mut("pipe-1").unwrap().step_status = StepStatus::Running;

    let state = Arc::new(Mutex::new(state));

    // Create real adapters (won't be called for ShellExited -> completion path)
    let session_adapter = TracedSession::new(TmuxAdapter::new());
    let agent_adapter = TracedAgent::new(ClaudeAgentAdapter::new(session_adapter.clone()));

    let (internal_tx, _internal_rx) = mpsc::channel::<Event>(100);
    let runtime = Arc::new(Runtime::new(
        RuntimeDeps {
            sessions: session_adapter,
            agents: agent_adapter,
            notifier: DesktopNotifyAdapter::new(),
            state: Arc::clone(&state),
        },
        SystemClock,
        RuntimeConfig {
            state_dir: dir_path.clone(),
            log_dir: dir_path.join("logs"),
        },
        internal_tx,
    ));

    let lock_path = dir_path.join("test.lock");
    let lock_file = std::fs::File::create(&lock_path).unwrap();

    let daemon = DaemonState {
        config: Config {
            state_dir: dir_path.clone(),
            socket_path: dir_path.join("test.sock"),
            lock_path,
            version_path: dir_path.join("test.version"),
            log_path: dir_path.join("test.log"),
            wal_path: wal_path.clone(),
            snapshot_path: dir_path.join("test.snapshot"),
            workspaces_path: dir_path.join("workspaces"),
            logs_path: dir_path.join("logs"),
        },
        lock_file,
        state,
        runtime,
        event_bus,
        start_time: std::time::Instant::now(),
        orphans: Arc::new(Mutex::new(Vec::new())),
    };

    (daemon, event_reader, wal_path)
}

/// Helper to create a Config pointing at a temp directory.
fn test_config(dir: &Path) -> Config {
    Config {
        state_dir: dir.to_path_buf(),
        socket_path: dir.join("test.sock"),
        lock_path: dir.join("test.lock"),
        version_path: dir.join("test.version"),
        log_path: dir.join("test.log"),
        wal_path: dir.join("test.wal"),
        snapshot_path: dir.join("test.snapshot"),
        workspaces_path: dir.join("workspaces"),
        logs_path: dir.join("logs"),
    }
}

/// Helper to create a runtime for reconciliation tests.
fn setup_reconcile_runtime(dir_path: &Path) -> (Arc<DaemonRuntime>, TracedSession<TmuxAdapter>) {
    let session_adapter = TracedSession::new(TmuxAdapter::new());
    let agent_adapter = TracedAgent::new(ClaudeAgentAdapter::new(session_adapter.clone()));
    let (internal_tx, _internal_rx) = mpsc::channel::<Event>(100);

    let state = Arc::new(Mutex::new(MaterializedState::default()));
    let runtime = Arc::new(Runtime::new(
        RuntimeDeps {
            sessions: session_adapter.clone(),
            agents: agent_adapter,
            notifier: DesktopNotifyAdapter::new(),
            state: Arc::clone(&state),
        },
        SystemClock,
        RuntimeConfig {
            state_dir: dir_path.to_path_buf(),
            log_dir: dir_path.join("logs"),
        },
        internal_tx,
    ));

    (runtime, session_adapter)
}

/// Helper to create a job with an agent_id in step_history and a session_id.
fn make_job_with_agent(id: &str, step: &str, agent_uuid: &str, session_id: &str) -> Job {
    Job {
        id: id.to_string(),
        name: "test-job".to_string(),
        kind: "test".to_string(),
        namespace: "proj".to_string(),
        step: step.to_string(),
        step_status: StepStatus::Running,
        step_started_at: std::time::Instant::now(),
        step_history: vec![StepRecord {
            name: step.to_string(),
            started_at_ms: 1000,
            finished_at_ms: None,
            outcome: StepOutcome::Running,
            agent_id: Some(agent_uuid.to_string()),
            agent_name: Some("test-agent".to_string()),
        }],
        vars: HashMap::new(),
        runbook_hash: "abc123".to_string(),
        cwd: PathBuf::from("/tmp/project"),
        workspace_id: None,
        workspace_path: None,
        session_id: Some(session_id.to_string()),
        created_at: std::time::Instant::now(),
        error: None,
        action_tracker: Default::default(),
        cancelling: false,
        total_retries: 0,
        step_visits: HashMap::new(),
        cron_name: None,
        idle_grace_log_size: None,
        last_nudge_at: None,
    }
}
