// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_daemon::{AgentSummary, JobSummary, SessionSummary};

fn job(id: &str, name: &str) -> JobSummary {
    JobSummary {
        id: id.to_string(),
        name: name.to_string(),
        kind: String::new(),
        step: String::new(),
        step_status: String::new(),
        created_at_ms: 0,
        updated_at_ms: 0,
        namespace: String::new(),
        retry_count: 0,
    }
}

fn agent(id: &str, name: Option<&str>) -> AgentSummary {
    AgentSummary {
        job_id: String::new(),
        step_name: String::new(),
        agent_id: id.to_string(),
        agent_name: name.map(String::from),
        namespace: None,
        status: String::new(),
        files_read: 0,
        files_written: 0,
        commands_run: 0,
        exit_reason: None,
        updated_at_ms: 0,
    }
}

fn session(id: &str) -> SessionSummary {
    SessionSummary {
        id: id.to_string(),
        namespace: String::new(),
        job_id: None,
        updated_at_ms: 0,
    }
}

#[test]
fn exact_match_job() {
    let jobs = vec![job("abc12345", "my-job")];
    let result = resolve_from_lists("abc12345", &jobs, &[], &[]);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].kind, EntityKind::Job);
    assert_eq!(result[0].id, "abc12345");
    assert_eq!(result[0].label.as_deref(), Some("my-job"));
}

#[test]
fn exact_match_agent() {
    let agents = vec![agent("agent-001", Some("builder"))];
    let result = resolve_from_lists("agent-001", &[], &agents, &[]);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].kind, EntityKind::Agent);
    assert_eq!(result[0].id, "agent-001");
    assert_eq!(result[0].label.as_deref(), Some("builder"));
}

#[test]
fn exact_match_session() {
    let sessions = vec![session("sess-xyz")];
    let result = resolve_from_lists("sess-xyz", &[], &[], &sessions);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].kind, EntityKind::Session);
    assert_eq!(result[0].id, "sess-xyz");
    assert_eq!(result[0].label, None);
}

#[test]
fn prefix_match_single() {
    let jobs = vec![job("abc12345", "my-job")];
    let result = resolve_from_lists("abc", &jobs, &[], &[]);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].kind, EntityKind::Job);
    assert_eq!(result[0].id, "abc12345");
}

#[test]
fn prefix_match_multiple_across_types() {
    let jobs = vec![job("abc12345", "my-job")];
    let agents = vec![agent("abc67890", None)];
    let sessions = vec![session("abcdef01")];
    let result = resolve_from_lists("abc", &jobs, &agents, &sessions);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].kind, EntityKind::Job);
    assert_eq!(result[1].kind, EntityKind::Agent);
    assert_eq!(result[2].kind, EntityKind::Session);
}

#[test]
fn exact_match_takes_priority_over_prefix() {
    let jobs = vec![job("abc", "short-id"), job("abcdef", "long-id")];
    let result = resolve_from_lists("abc", &jobs, &[], &[]);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, "abc");
    assert_eq!(result[0].label.as_deref(), Some("short-id"));
}

#[test]
fn no_match_returns_empty() {
    let jobs = vec![job("xyz123", "other")];
    let result = resolve_from_lists("abc", &jobs, &[], &[]);
    assert!(result.is_empty());
}

#[test]
fn exact_match_across_types_returns_all_exact() {
    // Unlikely but possible: same ID in different entity types
    let jobs = vec![job("abc123", "pipe")];
    let agents = vec![agent("abc123", Some("agt"))];
    let result = resolve_from_lists("abc123", &jobs, &agents, &[]);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].kind, EntityKind::Job);
    assert_eq!(result[1].kind, EntityKind::Agent);
}
