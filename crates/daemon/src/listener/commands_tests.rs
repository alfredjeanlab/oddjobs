// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tempfile::tempdir;

use oj_storage::{MaterializedState, Wal};

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::handle_run_command;

/// Helper: create a temp project with a runbook TOML and return the project root path.
fn project_with_runbook(toml_content: &str) -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let runbook_dir = dir.path().join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(runbook_dir.join("test.toml"), toml_content).unwrap();
    dir
}

/// Helper: create an EventBus backed by a temp WAL.
fn test_event_bus(dir: &std::path::Path) -> EventBus {
    let wal_path = dir.join("test.wal");
    let wal = Wal::open(&wal_path, 0).unwrap();
    let (event_bus, _reader) = EventBus::new(wal);
    event_bus
}

#[tokio::test]
async fn shell_command_uses_command_name_as_pipeline_name() {
    let project = project_with_runbook(
        r#"
[command.deploy]
run = "echo deploying"
"#,
    );

    let wal_dir = tempdir().unwrap();
    let event_bus = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_run_command(
        project.path(),
        project.path(),
        "",
        "deploy",
        &[],
        &HashMap::new(),
        &event_bus,
        &state,
    )
    .await
    .unwrap();

    match result {
        Response::CommandStarted { pipeline_name, .. } => {
            assert_eq!(pipeline_name, "deploy");
        }
        other => panic!("expected CommandStarted, got {:?}", other),
    }
}

#[tokio::test]
async fn pipeline_command_uses_pipeline_name() {
    let project = project_with_runbook(
        r#"
[command.build]
run = { pipeline = "build-all" }

[pipeline.build-all]
input  = []

[[pipeline.build-all.step]]
name = "compile"
run = "make"
"#,
    );

    let wal_dir = tempdir().unwrap();
    let event_bus = test_event_bus(wal_dir.path());
    let state = Arc::new(Mutex::new(MaterializedState::default()));

    let result = handle_run_command(
        project.path(),
        project.path(),
        "",
        "build",
        &[],
        &HashMap::new(),
        &event_bus,
        &state,
    )
    .await
    .unwrap();

    match result {
        Response::CommandStarted { pipeline_name, .. } => {
            assert_eq!(pipeline_name, "build-all");
        }
        other => panic!("expected CommandStarted, got {:?}", other),
    }
}
