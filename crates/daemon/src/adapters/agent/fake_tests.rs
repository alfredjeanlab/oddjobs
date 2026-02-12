// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::JobId;
use std::path::PathBuf;

fn test_spawn_config(agent_id: &str) -> AgentConfig {
    AgentConfig::new(
        AgentId::new(agent_id),
        "claude",
        PathBuf::from("/workspace"),
        OwnerId::Job(JobId::default()),
    )
    .agent_name("claude")
    .prompt("Test")
    .job_name("test")
    .job_id("job-1")
    .project_path(PathBuf::from("/project"))
}

#[tokio::test]
async fn spawn_and_kill() {
    let adapter = FakeAgentAdapter::new();
    let (tx, _rx) = mpsc::channel(10);

    let config = test_spawn_config("test-agent").prompt("Test prompt");

    let handle = adapter.spawn(config, tx).await.unwrap();
    assert_eq!(handle.agent_id, AgentId::new("test-agent"));
    assert!(adapter.has_agent(&AgentId::new("test-agent")));

    adapter.kill(&AgentId::new("test-agent")).await.unwrap();
    assert!(!adapter.has_agent(&AgentId::new("test-agent")));
}

#[tokio::test]
async fn state_changes() {
    let adapter = FakeAgentAdapter::new();
    let (tx, mut rx) = mpsc::channel(10);

    let config = test_spawn_config("agent-1");

    adapter.spawn(config, tx).await.unwrap();

    // Initial state should be Working
    let state = adapter.get_state(&AgentId::new("agent-1")).await.unwrap();
    assert_eq!(state, AgentState::Working);

    // Set state to WaitingForInput
    adapter.set_agent_state(&AgentId::new("agent-1"), AgentState::WaitingForInput);
    let state = adapter.get_state(&AgentId::new("agent-1")).await.unwrap();
    assert_eq!(state, AgentState::WaitingForInput);

    // Emit a state change event
    adapter
        .emit_state_change(&AgentId::new("agent-1"), AgentState::Exited { exit_code: Some(0) })
        .await;

    let event = rx.recv().await.unwrap();
    match event {
        Event::AgentExited { id, exit_code, .. } => {
            assert_eq!(id, AgentId::new("agent-1"));
            assert_eq!(exit_code, Some(0));
        }
        _ => panic!("unexpected event: {:?}", event),
    }
}

#[tokio::test]
async fn error_injection() {
    let adapter = FakeAgentAdapter::new();
    let (tx, _rx) = mpsc::channel(10);

    adapter.set_spawn_error(AgentAdapterError::SpawnFailed("test error".to_string()));

    let config = test_spawn_config("agent-1");

    let result = adapter.spawn(config, tx).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn call_recording() {
    let adapter = FakeAgentAdapter::new();
    let (tx, _rx) = mpsc::channel(10);

    let config = AgentConfig::new(
        AgentId::new("agent-1"),
        "claude code",
        PathBuf::from("/workspace"),
        OwnerId::Job(JobId::default()),
    )
    .agent_name("claude")
    .prompt("Test")
    .job_name("test")
    .job_id("job-1")
    .project_path(PathBuf::from("/project"));

    adapter.spawn(config, tx).await.unwrap();
    adapter.send(&AgentId::new("agent-1"), "hello").await.unwrap();
    adapter.get_state(&AgentId::new("agent-1")).await.unwrap();
    adapter.kill(&AgentId::new("agent-1")).await.unwrap();

    let calls = adapter.calls();
    assert_eq!(calls.len(), 4);

    matches!(&calls[0], AgentCall::Spawn { agent_id, .. } if agent_id == &AgentId::new("agent-1"));
    matches!(&calls[1], AgentCall::Send { agent_id, input } if agent_id == &AgentId::new("agent-1") && input == "hello");
    matches!(&calls[2], AgentCall::GetState { agent_id } if agent_id == &AgentId::new("agent-1"));
    matches!(&calls[3], AgentCall::Kill { agent_id } if agent_id == &AgentId::new("agent-1"));
}
