// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for agent effects (spawn, send, kill).

use super::*;

#[tokio::test]
async fn spawn_agent_returns_session_created() {
    let harness = setup().await;

    let mut input = HashMap::new();
    input.insert("prompt".to_string(), "do the thing".to_string());
    input.insert("name".to_string(), "test-job".to_string());

    let result = harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-1"),
            agent_name: "builder".to_string(),
            owner: OwnerId::Job(JobId::new("job-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input,
            command: "claude".to_string(),
            env: vec![("FOO".to_string(), "bar".to_string())],
            cwd: None,
            session_config: HashMap::new(),
        })
        .await
        .unwrap();

    // Should return SessionCreated event
    assert!(matches!(result, Some(Event::SessionCreated { .. })));

    // Verify state was updated with the session
    let state = harness.executor.state();
    let state = state.lock();
    assert!(
        !state.sessions.is_empty(),
        "session should be tracked in state"
    );

    // Verify agent adapter was called
    let calls = harness.agents.calls();
    assert_eq!(calls.len(), 1);
}

#[tokio::test]
async fn spawn_agent_with_agent_run_owner() {
    let harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-2"),
            agent_name: "runner".to_string(),
            owner: OwnerId::AgentRun(AgentRunId::new("ar-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws2"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: Some(std::path::PathBuf::from("/tmp")),
            session_config: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(matches!(result, Some(Event::SessionCreated { .. })));

    // Verify the event has the correct owner
    if let Some(Event::SessionCreated { owner, .. }) = result {
        assert!(matches!(owner, OwnerId::AgentRun(_)));
    }
}

#[tokio::test]
async fn spawn_agent_error_propagates() {
    let harness = setup().await;

    // Inject a spawn error
    harness
        .agents
        .set_spawn_error(AgentError::SpawnFailed("test failure".to_string()));

    let result = harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-err"),
            agent_name: "builder".to_string(),
            owner: OwnerId::Job(JobId::new("job-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: None,
            session_config: HashMap::new(),
        })
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("test failure"));
}

// === SendToAgent / KillAgent tests ===

#[tokio::test]
async fn send_to_agent_delegates_to_adapter() {
    let harness = setup().await;

    // First spawn an agent so it exists
    harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-send"),
            agent_name: "builder".to_string(),
            owner: OwnerId::Job(JobId::new("job-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: None,
            session_config: HashMap::new(),
        })
        .await
        .unwrap();

    let result = harness
        .executor
        .execute(Effect::SendToAgent {
            agent_id: AgentId::new("agent-send"),
            input: "continue working".to_string(),
        })
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn send_to_agent_error_propagates() {
    let harness = setup().await;

    harness
        .agents
        .set_send_error(AgentError::NotFound("agent-missing".to_string()));

    let result = harness
        .executor
        .execute(Effect::SendToAgent {
            agent_id: AgentId::new("agent-missing"),
            input: "hello".to_string(),
        })
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("agent-missing"));
}

#[tokio::test]
async fn kill_agent_delegates_to_adapter() {
    let harness = setup().await;

    // Spawn an agent first so it can be killed
    harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::new("agent-kill"),
            agent_name: "builder".to_string(),
            owner: OwnerId::Job(JobId::new("job-1")),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: None,
            session_config: HashMap::new(),
        })
        .await
        .unwrap();

    let result = harness
        .executor
        .execute(Effect::KillAgent {
            agent_id: AgentId::new("agent-kill"),
        })
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn kill_agent_error_propagates() {
    let harness = setup().await;

    harness
        .agents
        .set_kill_error(AgentError::NotFound("agent-gone".to_string()));

    let result = harness
        .executor
        .execute(Effect::KillAgent {
            agent_id: AgentId::new("agent-gone"),
        })
        .await;

    assert!(result.is_err());
}
