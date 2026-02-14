// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[yare::parameterized(
    starting  = { CrewStatus::Starting },
    running   = { CrewStatus::Running },
    waiting   = { CrewStatus::Waiting },
    completed = { CrewStatus::Completed },
    failed    = { CrewStatus::Failed },
    escalated = { CrewStatus::Escalated },
)]
fn crew_status_roundtrips(status: CrewStatus) {
    let json = serde_json::to_string(&status).unwrap();
    let parsed: CrewStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(status, parsed);
}

#[yare::parameterized(
    starting  = { CrewStatus::Starting,  false },
    running   = { CrewStatus::Running,   false },
    waiting   = { CrewStatus::Waiting,   false },
    completed = { CrewStatus::Completed, true },
    failed    = { CrewStatus::Failed,    true },
    escalated = { CrewStatus::Escalated, false },
)]
fn terminal_iff_completed_or_failed(status: CrewStatus, expected: bool) {
    assert_eq!(status.is_terminal(), expected);
}

#[test]
fn crew_status_display() {
    assert_eq!(CrewStatus::Starting.to_string(), "starting");
    assert_eq!(CrewStatus::Running.to_string(), "running");
    assert_eq!(CrewStatus::Waiting.to_string(), "waiting");
    assert_eq!(CrewStatus::Completed.to_string(), "completed");
    assert_eq!(CrewStatus::Failed.to_string(), "failed");
    assert_eq!(CrewStatus::Escalated.to_string(), "escalated");
}

#[test]
fn crew_id_display() {
    let id = CrewId::from_string("abc-123");
    assert_eq!(id.to_string(), "abc-123");
    assert_eq!(id.as_str(), "abc-123");
}

#[test]
fn crew_id_equality() {
    let id = CrewId::from_string("test-id");
    assert_eq!(id, "test-id");
    assert_eq!(id, *"test-id");
}

#[test]
fn crew_attempts() {
    let mut run = Crew::builder()
        .id("test")
        .agent_name("test-agent")
        .command_name("test-cmd")
        .project("test-ns")
        .cwd("/tmp")
        .runbook_hash("abc123")
        .build();

    assert_eq!(run.actions.increment_attempt("idle", 0), 1);
    assert_eq!(run.actions.increment_attempt("idle", 0), 2);
    assert_eq!(run.actions.increment_attempt("exit", 0), 1);

    run.actions.reset_attempts();
    assert_eq!(run.actions.increment_attempt("idle", 0), 1);
}

#[test]
fn crew_serde_roundtrip() {
    let run = Crew::builder()
        .id("test-id")
        .agent_name("greeter")
        .command_name("greet")
        .project("my-project")
        .cwd("/home/user/project")
        .runbook_hash("deadbeef")
        .agent_id("uuid-123")
        .build();

    let json = serde_json::to_string(&run).unwrap();
    let deserialized: Crew = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.id, "test-id");
    assert_eq!(deserialized.agent_name, "greeter");
    assert_eq!(deserialized.status, CrewStatus::Running);
    assert_eq!(deserialized.agent_id.as_deref(), Some("uuid-123"));
}
