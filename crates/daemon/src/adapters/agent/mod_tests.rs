// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn agent_handle_accessors() {
    let handle = AgentHandle::new(AgentId::from_string("test-agent"));

    assert_eq!(handle.agent_id, AgentId::from_string("test-agent"));
}
