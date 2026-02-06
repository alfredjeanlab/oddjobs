// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::session::{FakeSessionAdapter, SessionCall};
use oj_core::{JobId, OwnerId};
use std::io::Write;
use std::time::Duration;
use tempfile::TempDir;

mod incremental_parser;
mod lifecycle;
mod parse_state;
mod watch_loop;

/// Append a JSONL line to a file (simulates real session log appends).
fn append_line(path: &Path, content: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{}", content).unwrap();
}

/// Create a temp dir with a `session.jsonl` containing `content`.
fn temp_log(content: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("session.jsonl");
    std::fs::write(&log_path, content).unwrap();
    (dir, log_path)
}

/// Create claude_base + workspace TempDirs, derive the project dir, and return
/// `(claude_base, workspace_dir, log_dir)`.
fn setup_claude_project(session_id: &str) -> (TempDir, TempDir, PathBuf) {
    let claude_base = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();
    let workspace_hash = project_dir_name(workspace_dir.path());
    let log_dir = claude_base.path().join("projects").join(&workspace_hash);
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join(format!("{session_id}.jsonl")),
        r#"{"type":"user","message":{"content":"hello"}}"#,
    )
    .unwrap();
    (claude_base, workspace_dir, log_dir)
}

/// Build a `WatcherConfig` with sensible defaults for tests.
fn test_watcher_config(log_session_id: &str, tmux_session_id: &str, project_path: &Path) -> WatcherConfig {
    WatcherConfig {
        agent_id: AgentId::new("test-agent"),
        log_session_id: log_session_id.to_string(),
        tmux_session_id: tmux_session_id.to_string(),
        project_path: project_path.to_path_buf(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::new("test-job")),
    }
}

/// Yield repeatedly so spawned tasks can make progress.
async fn yield_to_task() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

/// Construct `WatchLoopParams` for fallback-polling tests (no file watcher / log).
fn fallback_params(
    sessions: FakeSessionAdapter,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
) -> WatchLoopParams<FakeSessionAdapter> {
    WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-session".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::new("test-job")),
        log_path: None,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx: None,
    }
}

/// Construct `WatchLoopParams` for tests with a log file.
fn log_watch_params(
    log_path: PathBuf,
    sessions: FakeSessionAdapter,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
    file_rx: Option<mpsc::Receiver<()>>,
) -> WatchLoopParams<FakeSessionAdapter> {
    WatchLoopParams {
        agent_id: AgentId::new("test-agent"),
        tmux_session_id: "test-tmux".to_string(),
        process_name: "claude".to_string(),
        owner: OwnerId::Job(JobId::new("test-job")),
        log_path: Some(log_path),
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx: None,
        file_rx,
    }
}
