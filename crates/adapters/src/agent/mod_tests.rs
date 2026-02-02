// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn agent_handle_accessors() {
    let handle = AgentHandle::new(
        AgentId::new("test-agent"),
        "sess-1".to_string(),
        PathBuf::from("/workspace"),
    );

    assert_eq!(handle.agent_id, AgentId::new("test-agent"));
    assert_eq!(handle.session_id, "sess-1");
    assert_eq!(handle.workspace_path, PathBuf::from("/workspace"));
}
