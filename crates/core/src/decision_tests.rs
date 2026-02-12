// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::{AgentId, JobId};

#[yare::parameterized(
    question = { DecisionSource::Question },
    approval = { DecisionSource::Approval },
    gate     = { DecisionSource::Gate },
    error    = { DecisionSource::Error },
    dead     = { DecisionSource::Dead },
    idle     = { DecisionSource::Idle },
    plan     = { DecisionSource::Plan },
)]
fn decision_source_roundtrips(source: DecisionSource) {
    let json = serde_json::to_string(&source).unwrap();
    let parsed: DecisionSource = serde_json::from_str(&json).unwrap();
    assert_eq!(source, parsed);
}

#[yare::parameterized(
    idle     = { DecisionSource::Idle },
    question = { DecisionSource::Question },
    plan     = { DecisionSource::Plan },
    approval = { DecisionSource::Approval },
)]
fn alive_sources(source: DecisionSource) {
    assert!(source.is_alive_agent_source());
}

#[yare::parameterized(
    dead  = { DecisionSource::Dead },
    error = { DecisionSource::Error },
    gate  = { DecisionSource::Gate },
)]
fn dead_sources(source: DecisionSource) {
    assert!(!source.is_alive_agent_source());
}

#[yare::parameterized(
    question = { DecisionSource::Question },
    plan     = { DecisionSource::Plan },
)]
fn approval_cannot_supersede_question_or_plan(existing: DecisionSource) {
    assert!(!DecisionSource::Approval.should_supersede(&existing));
}

#[yare::parameterized(
    question = { DecisionSource::Question },
    approval = { DecisionSource::Approval },
    gate     = { DecisionSource::Gate },
    error    = { DecisionSource::Error },
    dead     = { DecisionSource::Dead },
    idle     = { DecisionSource::Idle },
    plan     = { DecisionSource::Plan },
)]
fn idle_supersedes_any(existing: DecisionSource) {
    assert!(DecisionSource::Idle.should_supersede(&existing));
}

#[yare::parameterized(
    question = { DecisionSource::Question },
    approval = { DecisionSource::Approval },
    gate     = { DecisionSource::Gate },
    error    = { DecisionSource::Error },
    dead     = { DecisionSource::Dead },
    idle     = { DecisionSource::Idle },
    plan     = { DecisionSource::Plan },
)]
fn same_source_supersedes_self(source: DecisionSource) {
    assert!(source.should_supersede(&source));
}

#[yare::parameterized(
    question = { DecisionSource::Question },
    gate     = { DecisionSource::Gate },
    error    = { DecisionSource::Error },
    dead     = { DecisionSource::Dead },
    idle     = { DecisionSource::Idle },
    plan     = { DecisionSource::Plan },
)]
fn non_approval_supersedes_approval(source: DecisionSource) {
    assert!(source.should_supersede(&DecisionSource::Approval));
}

#[test]
fn decision_option_serde_roundtrip() {
    let opt = DecisionOption::new("Retry").description("Try the step again").recommended();
    let json = serde_json::to_string(&opt).unwrap();
    let parsed: DecisionOption = serde_json::from_str(&json).unwrap();
    assert_eq!(opt, parsed);
}

#[test]
fn decision_option_minimal_serde() {
    let opt = DecisionOption::new("Skip");
    let json = serde_json::to_string(&opt).unwrap();
    assert!(!json.contains("description"));
    let parsed: DecisionOption = serde_json::from_str(&json).unwrap();
    assert_eq!(opt, parsed);
}

#[test]
fn decision_serde_roundtrip() {
    let decision = Decision {
        id: DecisionId::new("dec-123"),
        agent_id: AgentId::new("agent-1"),
        owner: JobId::new("job-1").into(),
        source: DecisionSource::Gate,
        context: "Gate failed with exit code 1".to_string(),
        options: vec![
            DecisionOption::new("Retry").recommended(),
            DecisionOption::new("Skip").description("Skip this step"),
        ],
        questions: None,
        choices: vec![],
        message: None,
        created_at_ms: 1_000_000,
        resolved_at_ms: None,
        superseded_by: None,
        project: "myproject".to_string(),
    };
    let json = serde_json::to_string(&decision).unwrap();
    let parsed: Decision = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.id, DecisionId::new("dec-123"));
    assert_eq!(parsed.owner, OwnerId::Job(JobId::new("job-1")));
    assert_eq!(parsed.source, DecisionSource::Gate);
    assert_eq!(parsed.options.len(), 2);
    assert!(!parsed.is_resolved());
}

#[test]
fn decision_is_resolved() {
    let mut decision = Decision {
        id: DecisionId::new("dec-1"),
        agent_id: AgentId::new("agent-1"),
        owner: JobId::new("job-1").into(),
        source: DecisionSource::Question,
        context: "What should we do?".to_string(),
        options: vec![],
        questions: None,
        choices: vec![],
        message: None,
        created_at_ms: 1_000_000,
        resolved_at_ms: None,
        superseded_by: None,
        project: String::new(),
    };
    assert!(!decision.is_resolved());

    decision.resolved_at_ms = Some(2_000_000);
    assert!(decision.is_resolved());
}

#[test]
fn decision_id_display() {
    let id = DecisionId::new("abc-123");
    assert_eq!(format!("{}", id), "abc-123");
    assert_eq!(id.as_str(), "abc-123");
}

#[test]
fn decision_id_from_conversions() {
    let id1: DecisionId = "test".into();
    let id2: DecisionId = String::from("test").into();
    assert_eq!(id1, id2);
    assert_eq!(id1, *"test");
}
