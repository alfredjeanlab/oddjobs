// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::adapters::agent::{AgentAdapter, AgentConfig};
use oj_core::{AgentId, JobId, OwnerId};
use std::path::PathBuf;
use tempfile::TempDir;

#[tokio::test]
async fn spawn_rejects_nonexistent_cwd() {
    let state_dir = TempDir::new().unwrap();
    let adapter = LocalAdapter::new(state_dir.path().to_path_buf());
    let (tx, _rx) = mpsc::channel(10);

    let project_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    let config = AgentConfig::new(
        AgentId::new("test-agent-1"),
        "claude code",
        workspace_dir.path().to_path_buf(),
        OwnerId::Job(JobId::default()),
    )
    .cwd(PathBuf::from("/nonexistent/path"))
    .prompt("Test prompt")
    .job_name("test-job")
    .job_id("job-1")
    .project_path(project_dir.path().to_path_buf());

    let result = adapter.spawn(config, tx).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("working directory does not exist"),
        "Expected error about working directory, got: {}",
        err
    );
}
