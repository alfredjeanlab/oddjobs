// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::agent::AgentSpawnConfig;
use oj_core::{AgentId, JobId, OwnerId};
use serial_test::{parallel, serial};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing_subscriber::fmt::MakeWriter;

/// A writer that captures log output for testing
#[derive(Clone, Default)]
struct CapturedLogs {
    logs: Arc<Mutex<Vec<u8>>>,
}

impl CapturedLogs {
    fn new() -> Self {
        Self::default()
    }

    fn contents(&self) -> String {
        let logs = self.logs.lock().unwrap();
        String::from_utf8_lossy(&logs).to_string()
    }
}

impl std::io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.logs.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CapturedLogs {
    type Writer = CapturedLogs;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Run a test with captured tracing output
fn with_tracing<F, Fut>(f: F) -> (String, Fut::Output)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future,
{
    let logs = CapturedLogs::new();
    let logs_clone = logs.clone();

    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs_clone)
        .with_ansi(false)
        .without_time()
        .finish();

    let result = tracing::subscriber::with_default(subscriber, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f())
    });

    (logs.contents(), result)
}

/// Assert that captured logs contain the expected substring
fn assert_log(logs: &str, label: &str, expected: &str) {
    assert!(logs.contains(expected), "Should log {label}. Logs:\n{logs}",);
}

/// Spawn a traced session, returning the fake adapter, traced wrapper, and session id
async fn spawn_traced_session() -> (
    crate::session::FakeSessionAdapter,
    TracedSession<crate::session::FakeSessionAdapter>,
    String,
) {
    let fake = crate::session::FakeSessionAdapter::default();
    let traced = TracedSession::new(fake.clone());
    let session_id = traced
        .spawn("test", Path::new("/tmp"), "echo", &[])
        .await
        .unwrap();
    (fake, traced, session_id)
}

fn test_spawn_config(cwd: Option<PathBuf>) -> AgentSpawnConfig {
    AgentSpawnConfig {
        agent_id: AgentId::new("test-agent-1"),
        agent_name: "claude".to_string(),
        command: "claude code".to_string(),
        env: vec![],
        workspace_path: PathBuf::from("/tmp"),
        cwd,
        prompt: "Test prompt".to_string(),
        job_name: "test-job".to_string(),
        job_id: "pipe-1".to_string(),
        project_root: PathBuf::from("/project"),
        session_config: std::collections::HashMap::new(),
        owner: OwnerId::Job(JobId::default()),
    }
}

/// Spawn a traced agent, returning the fake adapter, traced wrapper, and agent id
async fn spawn_traced_agent() -> (
    crate::agent::FakeAgentAdapter,
    TracedAgent<crate::agent::FakeAgentAdapter>,
    AgentId,
) {
    let fake = crate::agent::FakeAgentAdapter::new();
    let traced = TracedAgent::new(fake.clone());
    let (tx, _rx) = mpsc::channel(10);
    let config = test_spawn_config(None);
    traced.spawn(config, tx).await.unwrap();
    (fake, traced, AgentId::new("test-agent-1"))
}

// =============================================================================
// Tracing output verification tests
// =============================================================================

#[test]
#[serial(tracing)]
fn traced_session_spawn_logs_entry_and_completion() {
    let (logs, result) = with_tracing(|| async {
        let fake = crate::session::FakeSessionAdapter::default();
        let traced = TracedSession::new(fake);
        traced
            .spawn("test-agent", Path::new("/tmp"), "echo hello", &[])
            .await
    });

    assert!(result.is_ok(), "spawn should succeed: {:?}", result);
    assert_log(&logs, "span name", "session.spawn");
    assert_log(&logs, "session name", "test-agent");
    assert_log(&logs, "entry message", "starting");
    assert_log(&logs, "completion", "session created");
    assert_log(&logs, "timing", "elapsed_ms");
}

#[test]
#[serial(tracing)]
fn traced_session_send_logs_operation() {
    let (logs, _) = with_tracing(|| async {
        let (_, traced, session_id) = spawn_traced_session().await;
        traced.send(&session_id, "hello").await
    });

    assert_log(&logs, "send span", "session.send");
    assert_log(&logs, "send entry", "sending");
}

#[test]
#[serial(tracing)]
fn traced_session_kill_logs_operation() {
    let (logs, _) = with_tracing(|| async {
        let (_, traced, session_id) = spawn_traced_session().await;
        traced.kill(&session_id).await
    });

    assert_log(&logs, "kill span", "session.kill");
    assert_log(&logs, "kill completion", "killed");
}

#[test]
#[serial(tracing)]
fn traced_session_send_logs_error_on_failure() {
    let (logs, result) = with_tracing(|| async {
        let fake = crate::session::FakeSessionAdapter::default();
        let traced = TracedSession::new(fake);
        traced.send("nonexistent", "hello").await
    });

    assert!(result.is_err());
    assert_log(&logs, "send failure", "send failed");
}

#[test]
#[serial(tracing)]
fn traced_session_kill_logs_warning_on_failure() {
    let (logs, result) = with_tracing(|| async {
        let fake = crate::session::FakeSessionAdapter::default();
        let traced = TracedSession::new(fake);
        traced.kill("nonexistent").await
    });

    assert!(result.is_ok());
    assert_log(&logs, "kill completion", "killed");
}

// =============================================================================
// Delegation tests - verify traced wrapper delegates to inner adapter
// =============================================================================

#[tokio::test]
#[parallel(tracing)]
async fn traced_session_delegates_spawn_to_inner() {
    let fake = crate::session::FakeSessionAdapter::default();
    let traced = TracedSession::new(fake.clone());

    let session_id = traced
        .spawn(
            "my-agent",
            Path::new("/tmp"),
            "echo hello",
            &[("KEY".to_string(), "VALUE".to_string())],
        )
        .await
        .unwrap();

    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        crate::session::SessionCall::Spawn {
            name,
            cwd,
            cmd,
            env,
        } => {
            assert_eq!(name, "my-agent");
            assert_eq!(cwd, &PathBuf::from("/tmp"));
            assert_eq!(cmd, "echo hello");
            assert_eq!(env, &[("KEY".to_string(), "VALUE".to_string())]);
        }
        other => panic!("Expected Spawn call, got {:?}", other),
    }

    assert!(fake.get_session(&session_id).is_some());
}

// =============================================================================
// Additional coverage tests
// =============================================================================

#[tokio::test]
#[parallel(tracing)]
async fn traced_session_is_alive_delegates_to_inner() {
    let (fake, traced, session_id) = spawn_traced_session().await;

    assert!(traced.is_alive(&session_id).await.unwrap());
    fake.set_exited(&session_id, 0);
    assert!(!traced.is_alive(&session_id).await.unwrap());
}

#[tokio::test]
#[parallel(tracing)]
async fn traced_session_is_alive_returns_false_for_unknown() {
    let fake = crate::session::FakeSessionAdapter::default();
    let traced = TracedSession::new(fake);
    assert!(!traced.is_alive("unknown").await.unwrap());
}

#[tokio::test]
#[parallel(tracing)]
async fn traced_session_capture_output_delegates_to_inner() {
    let (fake, traced, session_id) = spawn_traced_session().await;
    fake.set_output(&session_id, vec!["line1".to_string(), "line2".to_string()]);

    let output = traced.capture_output(&session_id, 10).await.unwrap();
    assert!(output.contains("line1"));
    assert!(output.contains("line2"));
}

#[tokio::test]
#[parallel(tracing)]
async fn traced_session_capture_output_error_for_unknown() {
    let fake = crate::session::FakeSessionAdapter::default();
    let traced = TracedSession::new(fake);
    assert!(traced.capture_output("unknown", 10).await.is_err());
}

#[tokio::test]
#[parallel(tracing)]
async fn traced_session_is_process_running_delegates_to_inner() {
    let (fake, traced, session_id) = spawn_traced_session().await;

    assert!(traced
        .is_process_running(&session_id, "pattern")
        .await
        .unwrap());
    fake.set_process_running(&session_id, false);
    assert!(!traced
        .is_process_running(&session_id, "pattern")
        .await
        .unwrap());
}

#[tokio::test]
#[parallel(tracing)]
async fn traced_session_is_process_running_returns_false_for_unknown() {
    let fake = crate::session::FakeSessionAdapter::default();
    let traced = TracedSession::new(fake);
    assert!(!traced
        .is_process_running("unknown", "pattern")
        .await
        .unwrap());
}

// =============================================================================
// Agent adapter tests
// =============================================================================

#[test]
#[serial(tracing)]
fn traced_agent_spawn_logs_entry_and_completion() {
    let (logs, result) = with_tracing(|| async {
        let fake = crate::agent::FakeAgentAdapter::new();
        let traced = TracedAgent::new(fake);
        let (tx, _rx) = mpsc::channel(10);
        let config = test_spawn_config(None);
        traced.spawn(config, tx).await
    });

    assert!(result.is_ok(), "spawn should succeed: {:?}", result);
    assert_log(&logs, "span name", "agent.spawn");
    assert_log(&logs, "agent_id", "test-agent-1");
    assert_log(&logs, "entry message", "starting");
    assert_log(&logs, "completion", "agent spawned");
    assert_log(&logs, "timing", "elapsed_ms");
}

#[test]
#[serial(tracing)]
fn traced_agent_send_logs_operation() {
    let (logs, _) = with_tracing(|| async {
        let (_, traced, agent_id) = spawn_traced_agent().await;
        traced.send(&agent_id, "hello").await
    });

    assert_log(&logs, "send span", "agent.send");
    assert_log(&logs, "send entry", "sending");
}

#[test]
#[serial(tracing)]
fn traced_agent_kill_logs_operation() {
    let (logs, _) = with_tracing(|| async {
        let (_, traced, agent_id) = spawn_traced_agent().await;
        traced.kill(&agent_id).await
    });

    assert_log(&logs, "kill span", "agent.kill");
    assert_log(&logs, "kill completion", "killed");
}

#[tokio::test]
#[parallel(tracing)]
async fn traced_agent_delegates_spawn_to_inner() {
    let fake = crate::agent::FakeAgentAdapter::new();
    let traced = TracedAgent::new(fake.clone());
    let (tx, _rx) = mpsc::channel(10);

    let config = test_spawn_config(None);
    let handle = traced.spawn(config, tx).await.unwrap();

    assert_eq!(handle.agent_id, AgentId::new("test-agent-1"));

    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        crate::agent::AgentCall::Spawn { agent_id, command } => {
            assert_eq!(agent_id, &AgentId::new("test-agent-1"));
            assert_eq!(command, "claude code");
        }
        other => panic!("Expected Spawn call, got {:?}", other),
    }
}

#[tokio::test]
#[parallel(tracing)]
async fn traced_agent_delegates_kill_to_inner() {
    let (fake, traced, agent_id) = spawn_traced_agent().await;
    fake.clear_calls();

    traced.kill(&agent_id).await.unwrap();

    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        crate::agent::AgentCall::Kill { agent_id } => {
            assert_eq!(agent_id, &AgentId::new("test-agent-1"));
        }
        other => panic!("Expected Kill call, got {:?}", other),
    }
}
