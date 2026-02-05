// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn owner_id_job_serde_roundtrip() {
    let owner = OwnerId::Job(JobId::new("job-123"));
    let json = serde_json::to_string(&owner).unwrap();
    assert!(json.contains(r#""type":"job""#));
    assert!(json.contains(r#""id":"job-123""#));

    let parsed: OwnerId = serde_json::from_str(&json).unwrap();
    assert_eq!(owner, parsed);
}

#[test]
fn owner_id_agent_run_serde_roundtrip() {
    let owner = OwnerId::AgentRun(AgentRunId::new("ar-456"));
    let json = serde_json::to_string(&owner).unwrap();
    assert!(json.contains(r#""type":"agent_run""#));
    assert!(json.contains(r#""id":"ar-456""#));

    let parsed: OwnerId = serde_json::from_str(&json).unwrap();
    assert_eq!(owner, parsed);
}

#[test]
fn owner_id_json_format() {
    let job_owner = OwnerId::job(JobId::new("p1"));
    let json: serde_json::Value = serde_json::to_value(&job_owner).unwrap();
    assert_eq!(json["type"], "job");
    assert_eq!(json["id"], "p1");

    let ar_owner = OwnerId::agent_run(AgentRunId::new("ar1"));
    let json: serde_json::Value = serde_json::to_value(&ar_owner).unwrap();
    assert_eq!(json["type"], "agent_run");
    assert_eq!(json["id"], "ar1");
}

#[test]
fn owner_id_equality() {
    let o1 = OwnerId::Job(JobId::new("p1"));
    let o2 = OwnerId::Job(JobId::new("p1"));
    let o3 = OwnerId::Job(JobId::new("p2"));
    let o4 = OwnerId::AgentRun(AgentRunId::new("p1"));

    assert_eq!(o1, o2);
    assert_ne!(o1, o3);
    assert_ne!(o1, o4);
}
