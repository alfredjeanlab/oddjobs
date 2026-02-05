// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::agent_run::AgentRunId;
use crate::job::JobId;

#[test]
fn agent_record_serde_roundtrip_job_owner() {
    let record = AgentRecord {
        agent_id: "agent-123".to_string(),
        agent_name: "worker".to_string(),
        owner: OwnerId::Job(JobId::new("job-456")),
        namespace: "myproject".to_string(),
        workspace_path: PathBuf::from("/tmp/ws-test"),
        session_id: Some("sess-789".to_string()),
        status: AgentRecordStatus::Running,
        created_at_ms: 1_000_000,
        updated_at_ms: 2_000_000,
    };

    let json = serde_json::to_string(&record).unwrap();
    let restored: AgentRecord = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.agent_id, "agent-123");
    assert_eq!(restored.agent_name, "worker");
    assert_eq!(restored.owner, OwnerId::Job(JobId::new("job-456")));
    assert_eq!(restored.namespace, "myproject");
    assert_eq!(restored.session_id.as_deref(), Some("sess-789"));
    assert_eq!(restored.status, AgentRecordStatus::Running);
    assert_eq!(restored.created_at_ms, 1_000_000);
    assert_eq!(restored.updated_at_ms, 2_000_000);
}

#[test]
fn agent_record_serde_roundtrip_agent_run_owner() {
    let record = AgentRecord {
        agent_id: "agent-abc".to_string(),
        agent_name: "fixer".to_string(),
        owner: OwnerId::AgentRun(AgentRunId::new("ar-def")),
        namespace: "".to_string(),
        workspace_path: PathBuf::from("/tmp/ws-fix"),
        session_id: None,
        status: AgentRecordStatus::Starting,
        created_at_ms: 500_000,
        updated_at_ms: 500_000,
    };

    let json = serde_json::to_string(&record).unwrap();
    let restored: AgentRecord = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.owner, OwnerId::AgentRun(AgentRunId::new("ar-def")));
    assert!(restored.session_id.is_none());
    assert_eq!(restored.status, AgentRecordStatus::Starting);
}

#[test]
fn agent_record_status_variants() {
    let statuses = vec![
        (AgentRecordStatus::Starting, "\"starting\""),
        (AgentRecordStatus::Running, "\"running\""),
        (AgentRecordStatus::Idle, "\"idle\""),
        (AgentRecordStatus::Exited, "\"exited\""),
        (AgentRecordStatus::Gone, "\"gone\""),
    ];

    for (status, expected_json) in statuses {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, expected_json);
        let restored: AgentRecordStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, status);
    }
}
