// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{Pipeline, PipelineConfig, StepOutcome, SystemClock};
use std::collections::HashMap;
use tempfile::TempDir;

fn test_pipeline() -> Pipeline {
    let config = PipelineConfig {
        id: "test-pipeline-001".to_string(),
        name: "test-pipeline".to_string(),
        kind: "deploy".to_string(),
        vars: HashMap::from([("branch".to_string(), "main".to_string())]),
        runbook_hash: "abc123".to_string(),
        cwd: PathBuf::from("/tmp/test"),
        initial_step: "build".to_string(),
        namespace: "myproject".to_string(),
        cron_name: None,
    };
    Pipeline::new(config, &SystemClock)
}

#[test]
fn write_produces_valid_json() {
    let dir = TempDir::new().unwrap();
    let writer = BreadcrumbWriter::new(dir.path().to_path_buf());
    let pipeline = test_pipeline();

    writer.write(&pipeline);

    let path = log_paths::breadcrumb_path(dir.path(), "test-pipeline-001");
    assert!(path.exists());

    let content = std::fs::read_to_string(&path).unwrap();
    let breadcrumb: Breadcrumb = serde_json::from_str(&content).unwrap();

    assert_eq!(breadcrumb.pipeline_id, "test-pipeline-001");
    assert_eq!(breadcrumb.project, "myproject");
    assert_eq!(breadcrumb.kind, "deploy");
    assert_eq!(breadcrumb.name, "test-pipeline");
    assert_eq!(breadcrumb.current_step, "build");
    assert_eq!(breadcrumb.step_status, "pending");
    assert_eq!(breadcrumb.vars.get("branch").unwrap(), "main");
    assert!(!breadcrumb.updated_at.is_empty());
    assert_eq!(breadcrumb.runbook_hash, "abc123");
    assert_eq!(breadcrumb.cwd.as_deref(), Some(Path::new("/tmp/test")));
}

#[test]
fn delete_removes_file() {
    let dir = TempDir::new().unwrap();
    let writer = BreadcrumbWriter::new(dir.path().to_path_buf());
    let pipeline = test_pipeline();

    writer.write(&pipeline);

    let path = log_paths::breadcrumb_path(dir.path(), "test-pipeline-001");
    assert!(path.exists());

    writer.delete("test-pipeline-001");
    assert!(!path.exists());
}

#[test]
fn delete_nonexistent_is_noop() {
    let dir = TempDir::new().unwrap();
    let writer = BreadcrumbWriter::new(dir.path().to_path_buf());
    // Should not panic
    writer.delete("nonexistent-pipeline");
}

#[test]
fn scan_breadcrumbs_finds_files() {
    let dir = TempDir::new().unwrap();
    let writer = BreadcrumbWriter::new(dir.path().to_path_buf());

    let mut p1 = test_pipeline();
    p1.id = "pipeline-aaa".to_string();
    writer.write(&p1);

    let mut p2 = test_pipeline();
    p2.id = "pipeline-bbb".to_string();
    p2.step = "deploy".to_string();
    writer.write(&p2);

    let breadcrumbs = scan_breadcrumbs(dir.path());
    assert_eq!(breadcrumbs.len(), 2);

    let ids: Vec<&str> = breadcrumbs.iter().map(|b| b.pipeline_id.as_str()).collect();
    assert!(ids.contains(&"pipeline-aaa"));
    assert!(ids.contains(&"pipeline-bbb"));
}

#[test]
fn scan_skips_corrupt_files() {
    let dir = TempDir::new().unwrap();
    let writer = BreadcrumbWriter::new(dir.path().to_path_buf());

    // Write a valid breadcrumb
    let pipeline = test_pipeline();
    writer.write(&pipeline);

    // Write a corrupt breadcrumb file
    let corrupt_path = dir.path().join("corrupt-id.crumb.json");
    std::fs::write(&corrupt_path, "not valid json{{{").unwrap();

    let breadcrumbs = scan_breadcrumbs(dir.path());
    assert_eq!(breadcrumbs.len(), 1);
    assert_eq!(breadcrumbs[0].pipeline_id, "test-pipeline-001");
}

#[test]
fn scan_empty_directory() {
    let dir = TempDir::new().unwrap();
    let breadcrumbs = scan_breadcrumbs(dir.path());
    assert!(breadcrumbs.is_empty());
}

#[test]
fn scan_nonexistent_directory() {
    let breadcrumbs = scan_breadcrumbs(Path::new("/nonexistent/path"));
    assert!(breadcrumbs.is_empty());
}

#[test]
fn round_trip_write_scan() {
    let dir = TempDir::new().unwrap();
    let writer = BreadcrumbWriter::new(dir.path().to_path_buf());

    let mut pipeline = test_pipeline();
    pipeline.id = "rt-001".to_string();
    pipeline.namespace = "proj".to_string();
    pipeline.kind = "ci".to_string();
    pipeline.name = "my-build".to_string();
    // Push a new step record matching the current step
    pipeline.push_step("test", 2000);
    pipeline.step = "test".to_string();

    // Add an agent to the current step
    if let Some(record) = pipeline.step_history.last_mut() {
        record.agent_id = Some("rt-001-test".to_string());
    }
    pipeline.session_id = Some("oj-rt-001-test".to_string());

    writer.write(&pipeline);

    let breadcrumbs = scan_breadcrumbs(dir.path());
    assert_eq!(breadcrumbs.len(), 1);

    let b = &breadcrumbs[0];
    assert_eq!(b.pipeline_id, "rt-001");
    assert_eq!(b.project, "proj");
    assert_eq!(b.kind, "ci");
    assert_eq!(b.name, "my-build");
    assert_eq!(b.current_step, "test");
    assert_eq!(b.agents.len(), 1);
    assert_eq!(b.agents[0].agent_id, "rt-001-test");
    assert_eq!(b.agents[0].session_name.as_deref(), Some("oj-rt-001-test"));
}

#[test]
fn write_captures_agents_from_history() {
    let dir = TempDir::new().unwrap();
    let writer = BreadcrumbWriter::new(dir.path().to_path_buf());

    let mut pipeline = test_pipeline();
    // Simulate a pipeline with two steps, each with an agent
    pipeline.step_history = vec![
        oj_core::StepRecord {
            name: "build".to_string(),
            started_at_ms: 1000,
            finished_at_ms: Some(2000),
            outcome: StepOutcome::Completed,
            agent_id: Some("p-001-build".to_string()),
            agent_name: None,
        },
        oj_core::StepRecord {
            name: "test".to_string(),
            started_at_ms: 2000,
            finished_at_ms: None,
            outcome: StepOutcome::Running,
            agent_id: Some("p-001-test".to_string()),
            agent_name: None,
        },
    ];
    pipeline.step = "test".to_string();
    pipeline.session_id = Some("oj-p-001-test".to_string());

    writer.write(&pipeline);

    let breadcrumbs = scan_breadcrumbs(dir.path());
    let b = &breadcrumbs[0];

    assert_eq!(b.agents.len(), 2);
    // First agent (past step) should not have session_name
    assert_eq!(b.agents[0].agent_id, "p-001-build");
    assert!(b.agents[0].session_name.is_none());
    // Second agent (current step) should have session_name
    assert_eq!(b.agents[1].agent_id, "p-001-test");
    assert_eq!(b.agents[1].session_name.as_deref(), Some("oj-p-001-test"));
}
