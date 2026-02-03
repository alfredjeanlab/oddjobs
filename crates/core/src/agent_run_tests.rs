// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn agent_run_id_display() {
    let id = AgentRunId::new("abc-123");
    assert_eq!(id.to_string(), "abc-123");
    assert_eq!(id.as_str(), "abc-123");
}

#[test]
fn agent_run_id_equality() {
    let id = AgentRunId::new("test-id");
    assert_eq!(id, "test-id");
    assert_eq!(id, *"test-id");
}

#[test]
fn agent_run_status_terminal() {
    assert!(!AgentRunStatus::Starting.is_terminal());
    assert!(!AgentRunStatus::Running.is_terminal());
    assert!(!AgentRunStatus::Waiting.is_terminal());
    assert!(AgentRunStatus::Completed.is_terminal());
    assert!(AgentRunStatus::Failed.is_terminal());
    assert!(!AgentRunStatus::Escalated.is_terminal());
}

#[test]
fn agent_run_status_display() {
    assert_eq!(AgentRunStatus::Starting.to_string(), "starting");
    assert_eq!(AgentRunStatus::Running.to_string(), "running");
    assert_eq!(AgentRunStatus::Waiting.to_string(), "waiting");
    assert_eq!(AgentRunStatus::Completed.to_string(), "completed");
    assert_eq!(AgentRunStatus::Failed.to_string(), "failed");
    assert_eq!(AgentRunStatus::Escalated.to_string(), "escalated");
}

#[test]
fn agent_run_action_attempts() {
    let mut run = AgentRun {
        id: "test".to_string(),
        agent_name: "test-agent".to_string(),
        command_name: "test-cmd".to_string(),
        namespace: "test-ns".to_string(),
        cwd: PathBuf::from("/tmp"),
        runbook_hash: "abc123".to_string(),
        status: AgentRunStatus::Running,
        agent_id: None,
        session_id: None,
        error: None,
        created_at_ms: 1000,
        updated_at_ms: 1000,
        action_attempts: HashMap::new(),
        agent_signal: None,
        vars: HashMap::new(),
    };

    assert_eq!(run.increment_action_attempt("idle", 0), 1);
    assert_eq!(run.increment_action_attempt("idle", 0), 2);
    assert_eq!(run.increment_action_attempt("exit", 0), 1);

    run.reset_action_attempts();
    assert_eq!(run.increment_action_attempt("idle", 0), 1);
}

#[test]
fn agent_run_serde_roundtrip() {
    let run = AgentRun {
        id: "test-id".to_string(),
        agent_name: "greeter".to_string(),
        command_name: "greet".to_string(),
        namespace: "my-project".to_string(),
        cwd: PathBuf::from("/home/user/project"),
        runbook_hash: "deadbeef".to_string(),
        status: AgentRunStatus::Running,
        agent_id: Some("uuid-123".to_string()),
        session_id: Some("sess-456".to_string()),
        error: None,
        created_at_ms: 1000,
        updated_at_ms: 2000,
        action_attempts: HashMap::new(),
        agent_signal: None,
        vars: HashMap::from([("key".to_string(), "value".to_string())]),
    };

    let json = serde_json::to_string(&run).unwrap();
    let deserialized: AgentRun = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.id, "test-id");
    assert_eq!(deserialized.agent_name, "greeter");
    assert_eq!(deserialized.status, AgentRunStatus::Running);
    assert_eq!(deserialized.agent_id.as_deref(), Some("uuid-123"));
}
