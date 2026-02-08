// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::PathBuf;

use tempfile::tempdir;

use oj_core::{Clock, FakeClock};

use crate::protocol::Response;

use super::{handle_cron_once, handle_cron_restart, handle_cron_start, handle_cron_stop};

/// Helper: create a CronRecord with a deterministic timestamp from a FakeClock.
fn make_cron_record(
    clock: &FakeClock,
    name: &str,
    namespace: &str,
    status: &str,
    interval: &str,
    job_kind: &str,
) -> oj_storage::CronRecord {
    oj_storage::CronRecord {
        name: name.to_string(),
        namespace: namespace.to_string(),
        project_root: PathBuf::from("/fake"),
        runbook_hash: "fake-hash".to_string(),
        status: status.to_string(),
        interval: interval.to_string(),
        run_target: format!("job:{}", job_kind),
        started_at_ms: clock.epoch_ms(),
        last_fired_at_ms: None,
    }
}

/// Helper: create a temp project with a valid cron runbook.
fn project_with_cron() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    let runbook_dir = dir.path().join(".oj/runbooks");
    std::fs::create_dir_all(&runbook_dir).unwrap();
    std::fs::write(
        runbook_dir.join("test.hcl"),
        r#"
cron "nightly" {
  interval = "24h"
  run      = { job = "deploy" }
}

job "deploy" {
  step "run" {
    run = "echo deploying"
  }
}
"#,
    )
    .unwrap();
    dir
}

// ── Race fix: handle_cron_start applies state before responding ──────────

#[test]
fn start_applies_state_before_responding() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    // Before: no crons in state
    assert!(ctx.state.lock().crons.is_empty());

    let result = handle_cron_start(&ctx, project.path(), "", "nightly", false).unwrap();

    // Handler returns CronStarted
    assert!(
        matches!(result, Response::CronStarted { ref cron_name } if cron_name == "nightly"),
        "expected CronStarted, got {:?}",
        result
    );

    // Race fix: cron is visible in state immediately (no WAL processing needed)
    let state = ctx.state.lock();
    let cron = state
        .crons
        .get("nightly")
        .expect("cron should be in state after start");
    assert_eq!(cron.name, "nightly");
    assert_eq!(cron.status, "running");
    assert_eq!(cron.interval, "24h");
    assert_eq!(cron.run_target, "job:deploy");
    assert!(cron.started_at_ms > 0, "started_at_ms should be set");
}

#[test]
fn start_with_namespace_uses_scoped_key() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    let result = handle_cron_start(&ctx, project.path(), "my-project", "nightly", false).unwrap();

    assert!(
        matches!(result, Response::CronStarted { ref cron_name } if cron_name == "nightly"),
        "expected CronStarted, got {:?}",
        result
    );

    let state = ctx.state.lock();
    // Namespace-scoped key: "my-project/nightly"
    assert!(
        !state.crons.contains_key("nightly"),
        "bare key should not exist"
    );
    let cron = state
        .crons
        .get("my-project/nightly")
        .expect("scoped key should exist");
    assert_eq!(cron.name, "nightly");
    assert_eq!(cron.namespace, "my-project");
    assert_eq!(cron.status, "running");
}

#[test]
fn start_idempotent_overwrites_existing() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    // Pre-populate state with an existing cron at a known timestamp
    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "nightly".to_string(),
            make_cron_record(&clock, "nightly", "", "running", "12h", "old-job"),
        );
    }

    let old_started = ctx.state.lock().crons["nightly"].started_at_ms;
    assert_eq!(old_started, clock.epoch_ms());

    // Start again — should overwrite with fresh runbook data
    let result = handle_cron_start(&ctx, project.path(), "", "nightly", false).unwrap();

    assert!(matches!(result, Response::CronStarted { .. }));

    let state = ctx.state.lock();
    let cron = state.crons.get("nightly").unwrap();
    assert_eq!(cron.status, "running");
    // Interval updated from runbook (24h, not the old 12h)
    assert_eq!(cron.interval, "24h");
    assert_eq!(cron.run_target, "job:deploy");
    // started_at_ms refreshed (real wall clock, so just verify it's set)
    assert!(cron.started_at_ms > 0);
}

// ── Race fix: handle_cron_stop applies state before responding ───────────

#[test]
fn stop_applies_state_before_responding() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    // Pre-populate a running cron at a known FakeClock timestamp
    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "nightly".to_string(),
            make_cron_record(&clock, "nightly", "", "running", "1h", "deploy"),
        );
    }

    let result = handle_cron_stop(&ctx, "nightly", "", None, false).unwrap();

    assert_eq!(result, Response::Ok);

    // Race fix: status is "stopped" immediately (no WAL processing needed)
    let state = ctx.state.lock();
    let cron = state
        .crons
        .get("nightly")
        .expect("cron should still be in state after stop");
    assert_eq!(cron.status, "stopped");
    // started_at_ms preserved from the original FakeClock value
    assert_eq!(cron.started_at_ms, clock.epoch_ms());
}

#[test]
fn stop_with_namespace_uses_scoped_key() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "my-project/nightly".to_string(),
            make_cron_record(&clock, "nightly", "my-project", "running", "1h", "deploy"),
        );
    }

    let result = handle_cron_stop(&ctx, "nightly", "my-project", None, false).unwrap();

    assert_eq!(result, Response::Ok);

    let state = ctx.state.lock();
    let cron = state.crons.get("my-project/nightly").unwrap();
    assert_eq!(cron.status, "stopped");
}

// ── Race fix: start-then-stop sequence visible without WAL processing ────

#[test]
fn start_then_immediate_stop_both_visible() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    // Start cron
    let start_result = handle_cron_start(&ctx, project.path(), "", "nightly", false).unwrap();
    assert!(matches!(start_result, Response::CronStarted { .. }));

    // Immediately verify running
    assert_eq!(ctx.state.lock().crons["nightly"].status, "running");

    // Stop without WAL processing in between
    let stop_result = handle_cron_stop(&ctx, "nightly", "", None, false).unwrap();
    assert_eq!(stop_result, Response::Ok);

    // Immediately verify stopped
    assert_eq!(ctx.state.lock().crons["nightly"].status, "stopped");
}

#[test]
fn stop_preserves_last_fired_at() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    let fired_at = clock.epoch_ms() - 60_000; // fired 60s ago
    let mut record = make_cron_record(&clock, "nightly", "", "running", "1h", "deploy");
    record.last_fired_at_ms = Some(fired_at);
    {
        let mut state = ctx.state.lock();
        state.crons.insert("nightly".to_string(), record);
    }

    handle_cron_stop(&ctx, "nightly", "", None, false).unwrap();

    let state = ctx.state.lock();
    let cron = state.crons.get("nightly").unwrap();
    assert_eq!(cron.status, "stopped");
    // last_fired_at_ms is preserved (CronStopped only changes status)
    assert_eq!(cron.last_fired_at_ms, Some(fired_at));
}

// ── Stop --all ───────────────────────────────────────────────────────────

#[test]
fn stop_all_stops_running_crons_in_namespace() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    // Pre-populate state with two running crons and one stopped cron
    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "my-project/nightly".to_string(),
            make_cron_record(&clock, "nightly", "my-project", "running", "24h", "deploy"),
        );
        state.crons.insert(
            "my-project/hourly".to_string(),
            make_cron_record(&clock, "hourly", "my-project", "running", "1h", "sync"),
        );
        state.crons.insert(
            "my-project/weekly".to_string(),
            make_cron_record(&clock, "weekly", "my-project", "stopped", "168h", "cleanup"),
        );
    }

    let result = handle_cron_stop(&ctx, "", "my-project", None, true).unwrap();

    match result {
        Response::CronsStopped { stopped, skipped } => {
            assert_eq!(stopped.len(), 2);
            assert!(stopped.contains(&"nightly".to_string()));
            assert!(stopped.contains(&"hourly".to_string()));
            assert!(skipped.is_empty());
        }
        other => panic!("expected CronsStopped, got {:?}", other),
    }

    // Verify all running crons are now stopped
    let state = ctx.state.lock();
    assert_eq!(state.crons["my-project/nightly"].status, "stopped");
    assert_eq!(state.crons["my-project/hourly"].status, "stopped");
    // Already-stopped cron is unchanged
    assert_eq!(state.crons["my-project/weekly"].status, "stopped");
}

#[test]
fn stop_all_only_affects_matching_namespace() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "proj-a/nightly".to_string(),
            make_cron_record(&clock, "nightly", "proj-a", "running", "24h", "deploy"),
        );
        state.crons.insert(
            "proj-b/nightly".to_string(),
            make_cron_record(&clock, "nightly", "proj-b", "running", "24h", "deploy"),
        );
    }

    let result = handle_cron_stop(&ctx, "", "proj-a", None, true).unwrap();

    match result {
        Response::CronsStopped { stopped, skipped } => {
            assert_eq!(stopped, vec!["nightly".to_string()]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected CronsStopped, got {:?}", other),
    }

    // proj-b cron should still be running
    let state = ctx.state.lock();
    assert_eq!(state.crons["proj-b/nightly"].status, "running");
}

// ── Existing restart tests ───────────────────────────────────────────────

#[test]
fn restart_without_runbook_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

    let result = handle_cron_restart(&ctx, std::path::Path::new("/fake"), "", "nightly").unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}

#[test]
fn restart_stops_existing_then_starts() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

    // Put a running cron in state so the restart path emits a stop event
    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "nightly".to_string(),
            oj_storage::CronRecord {
                name: "nightly".to_string(),
                namespace: String::new(),
                project_root: PathBuf::from("/fake"),
                runbook_hash: "fake-hash".to_string(),
                status: "running".to_string(),
                interval: "1h".to_string(),
                run_target: "job:deploy".to_string(),
                started_at_ms: 0,
                last_fired_at_ms: None,
            },
        );
    }

    // Restart with no runbook on disk — the stop event is emitted but start
    // fails because the runbook is missing.  This proves the stop path ran.
    let result = handle_cron_restart(&ctx, std::path::Path::new("/fake"), "", "nightly").unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error after stop, got {:?}",
        result
    );
}

#[test]
fn restart_with_valid_runbook_returns_started() {
    let dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(dir.path());

    let project = project_with_cron();

    // Put existing cron in state
    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "nightly".to_string(),
            oj_storage::CronRecord {
                name: "nightly".to_string(),
                namespace: String::new(),
                project_root: project.path().to_path_buf(),
                runbook_hash: "old-hash".to_string(),
                status: "running".to_string(),
                interval: "24h".to_string(),
                run_target: "job:deploy".to_string(),
                started_at_ms: 0,
                last_fired_at_ms: None,
            },
        );
    }

    let result = handle_cron_restart(&ctx, project.path(), "", "nightly").unwrap();

    assert!(
        matches!(result, Response::CronStarted { ref cron_name } if cron_name == "nightly"),
        "expected CronStarted response, got {:?}",
        result
    );
}

// ── CronOnce tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn once_with_wrong_project_root_falls_back_to_namespace() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    // Pre-populate state with a cron that knows the real project root,
    // simulating `--project town` where the daemon already tracks the namespace.
    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "my-project/nightly".to_string(),
            oj_storage::CronRecord {
                name: "nightly".to_string(),
                namespace: "my-project".to_string(),
                project_root: project.path().to_path_buf(),
                runbook_hash: "fake-hash".to_string(),
                status: "running".to_string(),
                interval: "24h".to_string(),
                run_target: String::new(),
                started_at_ms: 1_000,
                last_fired_at_ms: None,
            },
        );
    }

    // Call handle_cron_once with a wrong project_root (simulating --project
    // from a different directory). The handler should fall back to the known
    // project root for namespace "my-project".
    let result = handle_cron_once(
        &ctx,
        std::path::Path::new("/wrong/path"),
        "my-project",
        "nightly",
    )
    .await
    .unwrap();

    assert!(
        matches!(result, Response::CommandStarted { .. }),
        "expected CommandStarted from namespace fallback, got {:?}",
        result
    );
}

#[tokio::test]
async fn once_without_runbook_returns_error() {
    let wal_dir = tempdir().unwrap();
    let ctx = super::super::test_ctx(wal_dir.path());

    let result = handle_cron_once(&ctx, std::path::Path::new("/fake"), "", "nightly")
        .await
        .unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}
