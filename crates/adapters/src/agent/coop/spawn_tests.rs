// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn test_prepare_workspace() {
    let workspace_dir = TempDir::new().unwrap();

    prepare_workspace(workspace_dir.path()).await.unwrap();

    // Workspace directory should exist
    assert!(workspace_dir.path().exists());

    // Should NOT write a CLAUDE.md (prompt is passed via CLI arg)
    let claude_md = workspace_dir.path().join("CLAUDE.md");
    assert!(!claude_md.exists());
}

#[yare::parameterized(
    adds_flag         = { "claude --dangerously-skip-permissions", "claude --dangerously-skip-permissions --allow-dangerously-skip-permissions" },
    already_present   = { "claude --dangerously-skip-permissions --allow-dangerously-skip-permissions", "claude --dangerously-skip-permissions --allow-dangerously-skip-permissions" },
    no_skip_flag      = { "claude --print", "claude --print" },
)]
fn augment_command(input: &str, expected: &str) {
    assert_eq!(crate::agent::augment_command_for_skip_permissions(input), expected);
}
