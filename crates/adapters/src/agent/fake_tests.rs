// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::collections::HashMap;
use std::path::PathBuf;

#[tokio::test]
async fn spawn_and_kill() {
    let adapter = FakeAgentAdapter::new();
    let (tx, _rx) = mpsc::channel(10);

    let config = AgentSpawnConfig {
        agent_id: AgentId::new("test-agent"),
        agent_name: "claude".to_string(),
        command: "claude".to_string(),
        env: vec![],
        workspace_path: PathBuf::from("/workspace"),
        cwd: None,
        prompt: "Test prompt".to_string(),
        job_name: "test".to_string(),
        job_id: "pipe-1".to_string(),
        project_root: PathBuf::from("/project"),
        session_config: HashMap::new(),
    };

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

    let config = AgentSpawnConfig {
        agent_id: AgentId::new("agent-1"),
        agent_name: "claude".to_string(),
        command: "claude".to_string(),
        env: vec![],
        workspace_path: PathBuf::from("/workspace"),
        cwd: None,
        prompt: "Test".to_string(),
        job_name: "test".to_string(),
        job_id: "pipe-1".to_string(),
        project_root: PathBuf::from("/project"),
        session_config: HashMap::new(),
    };

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
        .emit_state_change(
            &AgentId::new("agent-1"),
            AgentState::Exited { exit_code: Some(0) },
        )
        .await;

    let event = rx.recv().await.unwrap();
    match event {
        Event::AgentExited {
            agent_id,
            exit_code,
            ..
        } => {
            assert_eq!(agent_id, AgentId::new("agent-1"));
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

    let config = AgentSpawnConfig {
        agent_id: AgentId::new("agent-1"),
        agent_name: "claude".to_string(),
        command: "claude".to_string(),
        env: vec![],
        workspace_path: PathBuf::from("/workspace"),
        cwd: None,
        prompt: "Test".to_string(),
        job_name: "test".to_string(),
        job_id: "pipe-1".to_string(),
        project_root: PathBuf::from("/project"),
        session_config: HashMap::new(),
    };

    let result = adapter.spawn(config, tx).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn call_recording() {
    let adapter = FakeAgentAdapter::new();
    let (tx, _rx) = mpsc::channel(10);

    let config = AgentSpawnConfig {
        agent_id: AgentId::new("agent-1"),
        agent_name: "claude".to_string(),
        command: "claude code".to_string(),
        env: vec![],
        workspace_path: PathBuf::from("/workspace"),
        cwd: None,
        prompt: "Test".to_string(),
        job_name: "test".to_string(),
        job_id: "pipe-1".to_string(),
        project_root: PathBuf::from("/project"),
        session_config: HashMap::new(),
    };

    adapter.spawn(config, tx).await.unwrap();
    adapter
        .send(&AgentId::new("agent-1"), "hello")
        .await
        .unwrap();
    adapter.get_state(&AgentId::new("agent-1")).await.unwrap();
    adapter.kill(&AgentId::new("agent-1")).await.unwrap();

    let calls = adapter.calls();
    assert_eq!(calls.len(), 4);

    matches!(&calls[0], AgentCall::Spawn { agent_id, .. } if agent_id == &AgentId::new("agent-1"));
    matches!(&calls[1], AgentCall::Send { agent_id, input } if agent_id == &AgentId::new("agent-1") && input == "hello");
    matches!(&calls[2], AgentCall::GetState { agent_id } if agent_id == &AgentId::new("agent-1"));
    matches!(&calls[3], AgentCall::Kill { agent_id } if agent_id == &AgentId::new("agent-1"));
}
