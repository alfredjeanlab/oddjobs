// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn agent_id_display() {
    let id = AgentId::new("test-agent");
    assert_eq!(id.to_string(), "test-agent");
}

#[test]
fn agent_id_equality() {
    let id1 = AgentId::new("agent-1");
    let id2 = AgentId::new("agent-1");
    let id3 = AgentId::new("agent-2");

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
}

#[test]
fn agent_id_from_str() {
    let id: AgentId = "test".into();
    assert_eq!(id.as_str(), "test");
}

#[test]
fn agent_id_serde() {
    let id = AgentId::new("my-agent");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"my-agent\"");

    let parsed: AgentId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, id);
}

#[test]
fn agent_state_display() {
    assert_eq!(AgentState::Working.to_string(), "working");
    assert_eq!(AgentState::WaitingForInput.to_string(), "waiting for input");
    assert_eq!(
        AgentState::Failed(AgentError::Unauthorized).to_string(),
        "failed: unauthorized"
    );
    assert_eq!(
        AgentState::Exited { exit_code: Some(0) }.to_string(),
        "exited with code 0"
    );
    assert_eq!(AgentState::Exited { exit_code: None }.to_string(), "exited");
    assert_eq!(AgentState::SessionGone.to_string(), "session gone");
}

#[test]
fn agent_failure_display() {
    assert_eq!(AgentError::Unauthorized.to_string(), "unauthorized");
    assert_eq!(AgentError::OutOfCredits.to_string(), "out of credits");
    assert_eq!(AgentError::NoInternet.to_string(), "no internet connection");
    assert_eq!(AgentError::RateLimited.to_string(), "rate limited");
    assert_eq!(AgentError::Other("test".into()).to_string(), "test");
}

#[test]
fn agent_state_serde() {
    let state = AgentState::Failed(AgentError::RateLimited);
    let json = serde_json::to_string(&state).unwrap();
    let parsed: AgentState = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, state);
}
