// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn agent_summary_deserializes_job_owner() {
    let json = r#"{"owner":"job:j1","step_name":"build","agent_id":"a1","status":"running","files_read":0,"files_written":0,"commands_run":0,"exit_reason":null}"#;
    let summary: AgentSummary = serde_json::from_str(json).expect("deserialize failed");
    assert!(matches!(summary.owner, oj_core::OwnerId::Job(_)));
    assert_eq!(summary.step_name, "build");
}
