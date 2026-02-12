// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn agent_summary_job_id_default() {
    let json = r#"{"step_name":"build","agent_id":"a1","status":"running","files_read":0,"files_written":0,"commands_run":0,"exit_reason":null}"#;
    let summary: AgentSummary = serde_json::from_str(json).expect("deserialize failed");
    assert_eq!(summary.job_id, "");
    assert_eq!(summary.step_name, "build");
}
