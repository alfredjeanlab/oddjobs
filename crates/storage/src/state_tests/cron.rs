// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{Event, JobId};

#[test]
fn cron_started_creates_record() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        run_target: "job:cleanup".to_string(),
        namespace: "myns".to_string(),
    });

    let key = "myns/janitor";
    assert!(state.crons.contains_key(key));
    let record = &state.crons[key];
    assert_eq!(record.name, "janitor");
    assert_eq!(record.namespace, "myns");
    assert_eq!(record.status, "running");
    assert_eq!(record.interval, "30m");
    assert_eq!(record.run_target, "job:cleanup");
}

#[test]
fn cron_stopped_updates_status() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        run_target: "job:cleanup".to_string(),
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
        run_target: "job:cleanup".to_string(),
        namespace: String::new(),
    });

    // Re-start with different interval (simulates runbook update)
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "def456".to_string(),
        interval: "1h".to_string(),
        run_target: "job:cleanup".to_string(),
        namespace: String::new(),
    });

    assert_eq!(state.crons.len(), 1);
    assert_eq!(state.crons["janitor"].interval, "1h");
    assert_eq!(state.crons["janitor"].runbook_hash, "def456");
}

#[test]
fn cron_deleted_removes_record() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        run_target: "job:cleanup".to_string(),
        namespace: "myns".to_string(),
    });

    assert!(state.crons.contains_key("myns/janitor"));

    state.apply_event(&Event::CronStopped {
        cron_name: "janitor".to_string(),
        namespace: "myns".to_string(),
    });
    assert_eq!(state.crons["myns/janitor"].status, "stopped");

    state.apply_event(&Event::CronDeleted {
        cron_name: "janitor".to_string(),
        namespace: "myns".to_string(),
    });
    assert!(!state.crons.contains_key("myns/janitor"));
}

#[test]
fn cron_deleted_empty_namespace() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        run_target: "job:cleanup".to_string(),
        namespace: String::new(),
    });

    assert!(state.crons.contains_key("janitor"));

    state.apply_event(&Event::CronDeleted {
        cron_name: "janitor".to_string(),
        namespace: String::new(),
    });
    assert!(!state.crons.contains_key("janitor"));
}

#[test]
fn cron_deleted_nonexistent_is_noop() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronDeleted {
        cron_name: "nonexistent".to_string(),
        namespace: String::new(),
    });
    assert!(state.crons.is_empty());
}

#[test]
fn cron_fired_is_noop_for_state() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronFired {
        cron_name: "janitor".to_string(),
        job_id: JobId::new("pipe-123"),
        agent_run_id: None,
        namespace: String::new(),
    });
    // CronFired should not create a record if the cron doesn't exist
    assert!(state.crons.is_empty());
}

#[test]
fn cron_fired_updates_last_fired_at() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        run_target: "job:cleanup".to_string(),
        namespace: String::new(),
    });

    assert!(state.crons["janitor"].last_fired_at_ms.is_none());

    state.apply_event(&Event::CronFired {
        cron_name: "janitor".to_string(),
        job_id: JobId::new("pipe-123"),
        agent_run_id: None,
        namespace: String::new(),
    });

    assert!(state.crons["janitor"].last_fired_at_ms.is_some());
}

#[test]
fn cron_started_sets_started_at_ms() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        run_target: "job:cleanup".to_string(),
        namespace: String::new(),
    });

    assert!(state.crons["janitor"].started_at_ms > 0);
}

#[test]
fn cron_restart_preserves_last_fired_at() {
    let mut state = MaterializedState::default();
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        run_target: "job:cleanup".to_string(),
        namespace: String::new(),
    });

    state.apply_event(&Event::CronFired {
        cron_name: "janitor".to_string(),
        job_id: JobId::new("pipe-123"),
        agent_run_id: None,
        namespace: String::new(),
    });

    let fired_ms = state.crons["janitor"].last_fired_at_ms;
    assert!(fired_ms.is_some());

    // Re-emit CronStarted (daemon restart)
    state.apply_event(&Event::CronStarted {
        cron_name: "janitor".to_string(),
        project_root: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        interval: "30m".to_string(),
        run_target: "job:cleanup".to_string(),
        namespace: String::new(),
    });

    // last_fired_at should be preserved
    assert_eq!(state.crons["janitor"].last_fired_at_ms, fired_ms);
}
