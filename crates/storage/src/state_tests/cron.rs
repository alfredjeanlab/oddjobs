// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{Event, PipelineId};

#[test]
fn cron_started_creates_record() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        pipeline_name: "cleanup".to_string(),
        namespace: "myns".to_string(),
    });

    let key = "myns/janitor";
    assert!(state.crons.contains_key(key));
    let record = &state.crons[key];
    assert_eq!(record.name, "janitor");
    assert_eq!(record.namespace, "myns");
    assert_eq!(record.status, "running");
    assert_eq!(record.interval, "30m");
    assert_eq!(record.pipeline_name, "cleanup");
}

#[test]
fn cron_stopped_updates_status() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        pipeline_name: "cleanup".to_string(),
        namespace: String::new(),
    });

    assert_eq!(state.crons["janitor"].status, "running");

    state.apply_event(&Event::CronStopped {
        cron_name: "janitor".to_string(),
        namespace: String::new(),
    });

    assert_eq!(state.crons["janitor"].status, "stopped");
}

#[test]
fn cron_started_is_idempotent() {
    let mut state = MaterializedState::default();

    // Start with one interval
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        pipeline_name: "cleanup".to_string(),
        namespace: String::new(),
    });

    // Re-start with different interval (simulates runbook update)
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "def456".to_string(),
        interval: "1h".to_string(),
        pipeline_name: "cleanup".to_string(),
        namespace: String::new(),
    });

    assert_eq!(state.crons.len(), 1);
    assert_eq!(state.crons["janitor"].interval, "1h");
    assert_eq!(state.crons["janitor"].runbook_hash, "def456");
}

#[test]
fn cron_fired_is_noop_for_state() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronFired {
        cron_name: "janitor".to_string(),
        pipeline_id: PipelineId::new("pipe-123"),
        namespace: String::new(),
    });
    // CronFired should not create any state by itself
    assert!(state.crons.is_empty());
}
