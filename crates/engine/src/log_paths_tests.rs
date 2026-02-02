// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn agent_log_path_builds_expected_path() {
    let result = agent_log_path(Path::new("/state/logs"), "abc-123-def");
    assert_eq!(result, PathBuf::from("/state/logs/agent/abc-123-def.log"));
}

#[test]
fn agent_session_log_dir_builds_expected_path() {
    let result = agent_session_log_dir(Path::new("/state/logs"), "abc-123-def");
    assert_eq!(result, PathBuf::from("/state/logs/agent/abc-123-def"));
}
