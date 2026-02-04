// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::MaterializedState;
use oj_core::{Pipeline, PipelineConfig, SystemClock};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use tempfile::tempdir;

fn test_config(id: &str, name: &str) -> PipelineConfig {
    PipelineConfig {
        id: id.to_string(),
        name: name.to_string(),
        kind: "feature".to_string(),
        vars: HashMap::new(),
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/test/project"),
        initial_step: "init".to_string(),
        namespace: String::new(),
        cron_name: None,
    }
}

fn create_test_state() -> MaterializedState {
    let mut state = MaterializedState::default();

    let pipeline = Pipeline::new(test_config("pipe-1", "test-pipeline"), &SystemClock);

    state.pipelines.insert("pipe-1".to_string(), pipeline);
    state
}

#[test]
fn test_snapshot_save_and_load() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    let state = create_test_state();
    let snapshot = Snapshot::new(42, state);

    // Save
    snapshot.save(&path).unwrap();
    assert!(path.exists());

    // Load
    let loaded = Snapshot::load(&path).unwrap().unwrap();
    assert_eq!(loaded.seq, 42);
    assert_eq!(loaded.state.pipelines.len(), 1);
    assert!(loaded.state.pipelines.contains_key("pipe-1"));
}

#[test]
fn test_load_nonexistent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nonexistent.json");

    let result = Snapshot::load(&path).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_snapshot_atomic_write() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");
    let tmp_path = path.with_extension("tmp");

    let state = create_test_state();
    let snapshot = Snapshot::new(1, state);

    // Save
    snapshot.save(&path).unwrap();

    // Temp file should not exist after successful save
    assert!(!tmp_path.exists());
    // Main file should exist
    assert!(path.exists());
}

#[test]
fn test_snapshot_preserves_state() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    let mut state = MaterializedState::default();

    // Add multiple items
    for i in 0..3 {
        let mut config = test_config(&format!("pipe-{}", i), &format!("test-{}", i));
        config.vars = HashMap::from([("key".to_string(), format!("value-{}", i))]);
        let pipeline = Pipeline::new(config, &SystemClock);
        state.pipelines.insert(format!("pipe-{}", i), pipeline);
    }

    let snapshot = Snapshot::new(100, state);
    snapshot.save(&path).unwrap();

    let loaded = Snapshot::load(&path).unwrap().unwrap();
    assert_eq!(loaded.seq, 100);
    assert_eq!(loaded.state.pipelines.len(), 3);

    for i in 0..3 {
        let key = format!("pipe-{}", i);
        let pipeline = loaded.state.pipelines.get(&key).unwrap();
        assert_eq!(pipeline.name, format!("test-{}", i));
        assert_eq!(pipeline.vars.get("key"), Some(&format!("value-{}", i)));
    }
}

#[test]
fn test_load_corrupt_snapshot_returns_none_and_creates_bak() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    // Write garbage data
    let mut f = File::create(&path).unwrap();
    f.write_all(b"\xe5\x03\x01binary-garbage").unwrap();
    drop(f);

    let result = Snapshot::load(&path).unwrap();
    assert!(result.is_none());

    // Original file should be gone
    assert!(!path.exists());
    // .bak should exist with the corrupt content
    let bak = path.with_extension("bak");
    assert!(bak.exists());
}

#[test]
fn test_load_corrupt_snapshot_rotates_bak_files() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    // Simulate 4 corrupt loads — should keep at most 3 backups
    for i in 1..=4u8 {
        let mut f = File::create(&path).unwrap();
        f.write_all(&[i; 4]).unwrap();
        drop(f);

        let result = Snapshot::load(&path).unwrap();
        assert!(result.is_none());
    }

    // .bak (most recent = round 4)
    let bak1 = path.with_extension("bak");
    assert!(bak1.exists());
    assert_eq!(fs::read(&bak1).unwrap(), vec![4u8; 4]);

    // .bak.2 (round 3)
    let bak2 = path.with_extension("bak.2");
    assert!(bak2.exists());
    assert_eq!(fs::read(&bak2).unwrap(), vec![3u8; 4]);

    // .bak.3 (round 2)
    let bak3 = path.with_extension("bak.3");
    assert!(bak3.exists());
    assert_eq!(fs::read(&bak3).unwrap(), vec![2u8; 4]);

    // Round 1 was evicted
    let bak4 = path.with_extension("bak.4");
    assert!(!bak4.exists());
}

#[test]
fn test_snapshot_round_trip_with_action_attempts() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    let mut state = MaterializedState::default();
    let mut pipeline = Pipeline::new(test_config("pipe-1", "test-pipeline"), &SystemClock);

    // Populate action_attempts (previously caused serialization failure)
    pipeline.increment_action_attempt("on_idle", 0);
    pipeline.increment_action_attempt("on_idle", 0);
    pipeline.increment_action_attempt("on_fail", 1);

    state.pipelines.insert("pipe-1".to_string(), pipeline);

    let snapshot = Snapshot::new(50, state);
    snapshot.save(&path).unwrap();

    let loaded = Snapshot::load(&path).unwrap().unwrap();
    assert_eq!(loaded.seq, 50);

    let p = loaded.state.pipelines.get("pipe-1").unwrap();
    assert_eq!(p.get_action_attempt("on_idle", 0), 2);
    assert_eq!(p.get_action_attempt("on_fail", 1), 1);
    assert_eq!(p.get_action_attempt("unknown", 0), 0);
}
