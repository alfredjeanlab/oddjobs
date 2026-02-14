// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for agent effects (spawn, send, kill).

use super::*;

#[tokio::test]
async fn spawn_agent_returns_none_and_sends_session_created() {
    let mut harness = setup().await;

    let mut input = HashMap::new();
    input.insert("prompt".to_string(), "do the thing".to_string());
    input.insert("name".to_string(), "test-job".to_string());

    let result = harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::from_string("agent-1"),
            agent_name: "builder".to_string(),
            owner: JobId::from_string("job-1").into(),
            workspace_path: std::path::PathBuf::from("/tmp/ws"),
            input,
            command: "claude".to_string(),
            env: vec![("FOO".to_string(), "bar".to_string())],
            cwd: None,
            unset_env: vec![],
            resume: false,
            container: None,
        })
        .await
        .unwrap();

    // SpawnAgent is now deferred — returns None immediately
    assert!(result.is_none(), "expected None, got: {:?}", result);

    // AgentSpawned arrives via the event channel
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), harness.event_rx.recv())
        .await
        .expect("timed out waiting for event")
        .expect("channel closed");
    assert!(matches!(event, Event::AgentSpawned { .. }), "expected AgentSpawned, got: {:?}", event);

    // Verify agent adapter was called
    let calls = harness.agents.calls();
    assert_eq!(calls.len(), 1);
}

#[tokio::test]
async fn spawn_agent_with_crew_owner() {
    let mut harness = setup().await;

    let result = harness
        .executor
        .execute(Effect::SpawnAgent {
            agent_id: AgentId::from_string("agent-2"),
            agent_name: "runner".to_string(),
            owner: CrewId::from_string("run-1").into(),
            workspace_path: std::path::PathBuf::from("/tmp/ws2"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![],
            cwd: Some(std::path::PathBuf::from("/tmp")),
            unset_env: vec![],
            resume: false,
            container: None,
        })
        .await
        .unwrap();

    // Deferred — returns None
    assert!(result.is_none());

    // Receive AgentSpawned with correct owner
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), harness.event_rx.recv())
        .await
        .expect("timed out waiting for event")
        .expect("channel closed");
    if let Event::AgentSpawned { owner, .. } = &event {
        assert!(matches!(owner, OwnerId::Crew(_)));
    } else {
        panic!("expected AgentSpawned, got: {:?}", event);
    }
}

#[tokio::test]
async fn spawn_agent_error_sends_agent_spawn_failed() {
    let mut harness = setup().await;

    // Inject a spawn error
    harness.agents.set_spawn_error(AgentAdapterError::SpawnFailed("test failure".to_string()));

    let result = harness.executor.execute(spawn_agent("agent-err")).await;

    // Deferred — returns Ok(None) even on failure
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());

    // AgentSpawnFailed arrives via the event channel
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), harness.event_rx.recv())
        .await
        .expect("timed out waiting for event")
        .expect("channel closed");
    if let Event::AgentSpawnFailed { id, reason, .. } = &event {
        assert_eq!(id.as_str(), "agent-err");
        assert!(reason.contains("test failure"));
    } else {
        panic!("expected AgentSpawnFailed, got: {:?}", event);
    }
}

// === SendToAgent / KillAgent tests ===

#[tokio::test]
async fn send_to_agent_delegates_to_adapter() {
    let mut harness = setup().await;

    // First spawn an agent so it exists
    harness.executor.execute(spawn_agent("agent-send")).await.unwrap();

    // Drain the AgentSpawned event from spawn
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), harness.event_rx.recv()).await;

    let result = harness
        .executor
        .execute(Effect::SendToAgent {
            agent_id: AgentId::from_string("agent-send"),
            input: "continue working".to_string(),
        })
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn send_to_agent_error_is_fire_and_forget() {
    let harness = setup().await;

    harness.agents.set_send_error(AgentAdapterError::NotFound("agent-missing".to_string()));

    let result = harness
        .executor
        .execute(Effect::SendToAgent {
            agent_id: AgentId::from_string("agent-missing"),
            input: "hello".to_string(),
        })
        .await;

    // Deferred fire-and-forget: returns Ok(None) even on adapter failure
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn kill_agent_delegates_to_adapter() {
    let mut harness = setup().await;

    // Spawn an agent first so it can be killed
    harness.executor.execute(spawn_agent("agent-kill")).await.unwrap();

    // Drain the AgentSpawned event from spawn
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), harness.event_rx.recv()).await;

    let result = harness
        .executor
        .execute(Effect::KillAgent { agent_id: AgentId::from_string("agent-kill") })
        .await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn kill_agent_error_is_fire_and_forget() {
    let harness = setup().await;

    harness.agents.set_kill_error(AgentAdapterError::NotFound("agent-gone".to_string()));

    let result = harness
        .executor
        .execute(Effect::KillAgent { agent_id: AgentId::from_string("agent-gone") })
        .await;

    // Deferred fire-and-forget: returns Ok(None) even on adapter failure
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}
