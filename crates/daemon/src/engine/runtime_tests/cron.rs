// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron-related runtime tests

use super::*;
use crate::engine::runtime::handlers::cron::CronStatus;
use oj_core::RunTarget;

fn cron_runbook_with_hash() -> (String, serde_json::Value, String) {
    let runbook = test_runbook_cron_job(
        "janitor",
        "cleanup",
        "interval = \"30m\"",
        &[
            ("prune", "echo pruning", "on_done = { step = \"done\" }"),
            ("done", "echo finished", ""),
        ],
    );
    let (json, hash) = hash_runbook(&runbook);
    (runbook, json, hash)
}

/// Helper: emit RunbookLoaded to populate the engine's in-process cache.
pub(super) async fn load_runbook(
    ctx: &TestContext,
    runbook_json: &serde_json::Value,
    runbook_hash: &str,
) {
    ctx.runtime
        .handle_event(Event::RunbookLoaded {
            hash: runbook_hash.to_string(),
            version: 1,
            runbook: runbook_json.clone(),
        })
        .await
        .unwrap();
}

// ---- Test 1: cron_once_creates_job ----

#[tokio::test]
async fn cron_once_creates_job() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    // Populate runbook cache
    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronOnce event
    let job_id = JobId::from_string("cron-job-1");
    let events = ctx
        .runtime
        .handle_event(Event::CronOnce {
            cron: "janitor".to_string(),
            owner: job_id.clone().into(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            target: RunTarget::job("cleanup"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Job should be created
    let job = ctx.runtime.get_job("cron-job-1").expect("job should exist");
    assert_eq!(job.kind, "cleanup");
    assert_eq!(job.step, "prune");

    // CronFired tracking event should have been emitted
    let has_cron_fired =
        events.iter().any(|e| matches!(e, Event::CronFired { cron, .. } if cron == "janitor"));
    assert!(has_cron_fired, "CronFired event should be emitted");
}

// ---- Test 2: cron_start_sets_timer ----

#[tokio::test]
async fn cron_start_sets_timer() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronStarted
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "janitor".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::job("cleanup"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Cron state should be Running
    {
        let crons = ctx.runtime.cron_states.lock();
        let state = crons.get("janitor").expect("cron state should exist");
        assert_eq!(state.status, CronStatus::Running);
        assert!(matches!(state.target, RunTarget::Job(ref p) if p == "cleanup"));
        assert_eq!(state.interval, "30m");
    }

    // Timer should have been set
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    assert!(sched.has_timers(), "timer should be set after CronStarted");

    // Advance clock past 30m and check that cron timer fires
    ctx.clock.advance(std::time::Duration::from_secs(30 * 60 + 1));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        timer_ids.iter().any(|id| id.starts_with("cron:")),
        "cron timer should fire after interval: {:?}",
        timer_ids
    );
}

// ---- Test 3: cron_stop_cancels_timer ----

#[tokio::test]
async fn cron_stop_cancels_timer() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "janitor".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::job("cleanup"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Stop cron
    ctx.runtime
        .handle_event(Event::CronStopped { cron: "janitor".to_string(), project: String::new() })
        .await
        .unwrap();

    // Cron state should be Stopped
    {
        let crons = ctx.runtime.cron_states.lock();
        let state = crons.get("janitor").expect("cron state should exist");
        assert_eq!(state.status, CronStatus::Stopped);
    }

    // Timer should be cancelled (no timers remaining)
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(std::time::Duration::from_secs(30 * 60 + 1));
    let fired = sched.fired_timers(ctx.clock.now());
    let cron_timers: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } if id.as_str().starts_with("cron:") => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(cron_timers.is_empty(), "no cron timers should fire after stop: {:?}", cron_timers);
}

// ---- Test 4: cron_timer_fired_creates_job ----

#[tokio::test]
async fn cron_timer_fired_creates_job() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron (registers state + sets timer)
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "janitor".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::job("cleanup"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Simulate timer firing via TimerStart event
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("janitor", "") })
        .await
        .unwrap();

    // Job should have been created
    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1, "one job should be created");

    let job = jobs.values().next().unwrap();
    assert_eq!(job.kind, "cleanup");
    assert_eq!(job.step, "prune");

    // CronFired event should be in result
    let has_cron_fired =
        events.iter().any(|e| matches!(e, Event::CronFired { cron, .. } if cron == "janitor"));
    assert!(has_cron_fired, "CronFired event should be emitted");
}

// ---- Test 5: cron_timer_fired_reloads_runbook ----

#[tokio::test]
async fn cron_timer_fired_reloads_runbook() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "janitor".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::job("cleanup"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Modify runbook on disk (add a comment to change the hash)
    let modified_runbook = r#"
[cron.janitor]
interval = "30m"
run = { job = "cleanup" }

[job.cleanup]

[[job.cleanup.step]]
name = "prune"
run = "echo pruning v2"
on_done = { step = "done" }

[[job.cleanup.step]]
name = "done"
run = "echo finished v2"
"#;
    let runbook_path = ctx.project_path.join(".oj/runbooks/test.toml");
    std::fs::write(&runbook_path, modified_runbook).unwrap();

    // Fire timer — should reload runbook from disk
    ctx.runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("janitor", "") })
        .await
        .unwrap();

    // Verify runbook hash was updated in cron state
    let new_hash = {
        let crons = ctx.runtime.cron_states.lock();
        crons.get("janitor").unwrap().runbook_hash.clone()
    };
    assert_ne!(new_hash, runbook_hash, "runbook hash should change after modification");
}

// ---- Test 6: cron_once_job_steps_execute ----

#[tokio::test]
async fn cron_once_job_steps_execute() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronOnce
    let job_id = JobId::from_string("cron-exec-1");
    ctx.runtime
        .handle_event(Event::CronOnce {
            cron: "janitor".to_string(),
            owner: job_id.clone().into(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            target: RunTarget::job("cleanup"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Job should be at first step
    let job = ctx.runtime.get_job("cron-exec-1").unwrap();
    assert_eq!(job.step, "prune");

    // Simulate shell completion of first step
    ctx.runtime.handle_event(shell_ok("cron-exec-1", "prune")).await.unwrap();

    // Job should advance to "done" step
    let job = ctx.runtime.get_job("cron-exec-1").unwrap();
    assert_eq!(job.step, "done");

    // Complete the "done" step
    ctx.runtime.handle_event(shell_ok("cron-exec-1", "done")).await.unwrap();

    // Job should be terminal
    let job = ctx.runtime.get_job("cron-exec-1").unwrap();
    assert!(job.is_terminal(), "job should be terminal after all steps complete");
}

// ---- Test 7: cron_timer_fired_reschedules_timer ----

#[tokio::test]
async fn cron_timer_fired_reschedules_timer() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "janitor".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::job("cleanup"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer (simulates the first interval expiring)
    ctx.runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("janitor", "") })
        .await
        .unwrap();

    // After firing, the handler should reschedule the timer for the next interval.
    // Verify the scheduler has a new timer pending.
    let scheduler = ctx.runtime.executor.scheduler();
    let sched = scheduler.lock();
    assert!(sched.has_timers(), "timer should be rescheduled after cron fires");
}

// ---- Test 8: cron_timer_fired_when_stopped_is_noop ----

#[tokio::test]
async fn cron_timer_fired_when_stopped_is_noop() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start and immediately stop the cron
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "janitor".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::job("cleanup"),
            project: String::new(),
        })
        .await
        .unwrap();

    ctx.runtime
        .handle_event(Event::CronStopped { cron: "janitor".to_string(), project: String::new() })
        .await
        .unwrap();

    // Simulate a timer firing for the stopped cron (race condition scenario)
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("janitor", "") })
        .await
        .unwrap();

    // No job should be created
    let jobs = ctx.runtime.jobs();
    assert!(jobs.is_empty(), "no job should be created for a stopped cron");

    // No CronFired event should be emitted
    let has_cron_fired = events.iter().any(|e| matches!(e, Event::CronFired { .. }));
    assert!(!has_cron_fired, "no CronFired event should be emitted for stopped cron");
}

// ---- Test 9: cron_start_with_namespace ----

#[tokio::test]
async fn cron_start_with_namespace() {
    let (runbook, runbook_json, runbook_hash) = cron_runbook_with_hash();
    let ctx = setup_with_runbook(&runbook).await;

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron with a project
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "janitor".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::job("cleanup"),
            project: "myproject".to_string(),
        })
        .await
        .unwrap();

    // Cron state should include project (key is now scoped: "myproject/janitor")
    {
        let crons = ctx.runtime.cron_states.lock();
        let state = crons.get("myproject/janitor").expect("cron state should exist");
        assert_eq!(state.project, "myproject");
    }

    // Timer ID should include project prefix
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(std::time::Duration::from_secs(30 * 60 + 1));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        timer_ids.contains(&"cron:myproject/janitor"),
        "timer ID should include project: {:?}",
        timer_ids
    );
}
