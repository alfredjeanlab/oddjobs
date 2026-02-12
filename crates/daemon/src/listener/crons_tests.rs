// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::path::PathBuf;

use tempfile::tempdir;

use oj_core::{Clock, FakeClock};

use crate::protocol::Response;

use super::super::test_ctx;
use super::{handle_cron_once, handle_cron_restart, handle_cron_start, handle_cron_stop};

/// Helper: create a CronRecord with a deterministic timestamp from a FakeClock.
fn make_cron_record(
    clock: &FakeClock,
    name: &str,
    project: &str,
    status: &str,
    interval: &str,
    job_kind: &str,
) -> oj_storage::CronRecord {
    oj_storage::CronRecord {
        name: name.to_string(),
        project: project.to_string(),
        project_path: PathBuf::from("/fake"),
        runbook_hash: "fake-hash".to_string(),
        status: status.to_string(),
        interval: interval.to_string(),
        target: oj_core::RunTarget::job(job_kind),
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

// ── handle_cron_start: state visible immediately ─────────────────────────

#[yare::parameterized(
    basic          = { "",           false, "nightly",            ""           },
    with_namespace = { "my-project", false, "my-project/nightly", "my-project" },
    idempotent     = { "",           true,  "nightly",            ""           },
)]
fn cron_start_applies_state(
    project: &str,
    pre_populate: bool,
    expected_key: &str,
    expected_namespace: &str,
) {
    let project_dir = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

    if pre_populate {
        let clock = FakeClock::new();
        clock.set_epoch_ms(1_700_000_000_000);
        let mut state = ctx.state.lock();
        state.crons.insert(
            "nightly".to_string(),
            make_cron_record(&clock, "nightly", "", "running", "12h", "old-job"),
        );
    }

    let result = handle_cron_start(&ctx, project_dir.path(), project, "nightly", false).unwrap();
    assert!(
        matches!(result, Response::CronStarted { ref cron } if cron =="nightly"),
        "expected CronStarted, got {:?}",
        result
    );

    let state = ctx.state.lock();
    if !expected_namespace.is_empty() {
        assert!(!state.crons.contains_key("nightly"), "bare key should not exist");
    }
    let cron = state.crons.get(expected_key).expect("cron should be in state");
    assert_eq!(cron.name, "nightly");
    assert_eq!(cron.project, expected_namespace);
    assert_eq!(cron.status, "running");
    assert_eq!(cron.interval, "24h");
    assert_eq!(cron.target, oj_core::RunTarget::job("deploy"));
    assert!(cron.started_at_ms > 0, "started_at_ms should be set");
}

// ── handle_cron_stop: state visible immediately ──────────────────────────

#[yare::parameterized(
    basic                = { "nightly",            "nightly", "",           false },
    with_namespace       = { "my-project/nightly", "nightly", "my-project", false },
    preserves_last_fired = { "nightly",            "nightly", "",           true  },
)]
fn cron_stop_applies_state(state_key: &str, cron_name: &str, project: &str, set_last_fired: bool) {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

    let fired_at = if set_last_fired { Some(clock.epoch_ms() - 60_000) } else { None };
    let mut record = make_cron_record(&clock, cron_name, project, "running", "1h", "deploy");
    record.last_fired_at_ms = fired_at;
    {
        let mut state = ctx.state.lock();
        state.crons.insert(state_key.to_string(), record);
    }

    let result = handle_cron_stop(&ctx, cron_name, project, None, false).unwrap();
    assert_eq!(result, Response::Ok);

    let state = ctx.state.lock();
    let cron = state.crons.get(state_key).expect("cron should still be in state after stop");
    assert_eq!(cron.status, "stopped");
    assert_eq!(cron.started_at_ms, clock.epoch_ms());
    assert_eq!(cron.last_fired_at_ms, fired_at);
}

// ── Race fix: start-then-stop sequence visible without WAL processing ────

#[test]
fn start_then_immediate_stop_both_visible() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

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

// ── Stop --all ───────────────────────────────────────────────────────────

#[test]
fn stop_all_stops_running_crons_in_namespace() {
    let clock = FakeClock::new();
    clock.set_epoch_ms(1_700_000_000_000);

    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

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
    let ctx = test_ctx(wal_dir.path());

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

// ── Restart tests ────────────────────────────────────────────────────────

#[yare::parameterized(
    no_state_no_runbook    = { false, false, true  },
    with_state_no_runbook  = { true,  false, true  },
    with_state_and_runbook = { true,  true,  false },
)]
fn cron_restart_behavior(pre_populate: bool, use_valid_project: bool, expect_error: bool) {
    let project = if use_valid_project { Some(project_with_cron()) } else { None };
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    if pre_populate {
        let clock = FakeClock::new();
        let mut record = make_cron_record(&clock, "nightly", "", "running", "24h", "deploy");
        if let Some(ref p) = project {
            record.project_path = p.path().to_path_buf();
        }
        record.runbook_hash = "old-hash".to_string();
        ctx.state.lock().crons.insert("nightly".to_string(), record);
    }

    let project_path = project.as_ref().map_or(std::path::Path::new("/fake"), |p| p.path());
    let result = handle_cron_restart(&ctx, project_path, "", "nightly").unwrap();

    if expect_error {
        assert!(
            matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
            "expected error, got {:?}",
            result
        );
    } else {
        assert!(
            matches!(result, Response::CronStarted { ref cron } if cron =="nightly"),
            "expected CronStarted, got {:?}",
            result
        );
    }
}

// ── CronOnce tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn once_with_wrong_project_path_falls_back_to_namespace() {
    let project = project_with_cron();
    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

    // Pre-populate state with a cron that knows the real project path,
    // simulating `--project town` where the daemon already tracks the project.
    {
        let mut state = ctx.state.lock();
        state.crons.insert(
            "my-project/nightly".to_string(),
            oj_storage::CronRecord {
                name: "nightly".to_string(),
                project: "my-project".to_string(),
                project_path: project.path().to_path_buf(),
                runbook_hash: "fake-hash".to_string(),
                status: "running".to_string(),
                interval: "24h".to_string(),
                target: oj_core::RunTarget::job("deploy"),
                started_at_ms: 1_000,
                last_fired_at_ms: None,
            },
        );
    }

    // Call handle_cron_once with a wrong project_path (simulating --project
    // from a different directory). The handler should fall back to the known
    // project root for project "my-project".
    let result =
        handle_cron_once(&ctx, std::path::Path::new("/wrong/path"), "my-project", "nightly")
            .await
            .unwrap();

    assert!(
        matches!(result, Response::JobStarted { .. }),
        "expected JobStarted from project fallback, got {:?}",
        result
    );
}

#[tokio::test]
async fn once_without_runbook_returns_error() {
    let wal_dir = tempdir().unwrap();
    let ctx = test_ctx(wal_dir.path());

    let result =
        handle_cron_once(&ctx, std::path::Path::new("/fake"), "", "nightly").await.unwrap();

    assert!(
        matches!(result, Response::Error { ref message } if message.contains("no runbook found")),
        "expected runbook-not-found error, got {:?}",
        result
    );
}
