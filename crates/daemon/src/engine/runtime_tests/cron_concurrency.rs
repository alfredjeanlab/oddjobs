// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job-targeted cron concurrency tests

use super::*;

use super::cron::load_runbook;

// ---- Test 17: cron_job_concurrency_skip ----

#[tokio::test]
async fn cron_job_concurrency_skip() {
    let runbook = test_runbook_cron_job(
        "deployer",
        "deploy",
        "interval = \"10m\"\nconcurrency = 1",
        &[
            ("run", "echo deploying", "on_done = { step = \"done\" }"),
            ("done", "echo finished", ""),
        ],
    );
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject an active (non-terminal) job with cron_name = Some("deployer")
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobCreated {
                id: JobId::from_string("existing-job-1"),
                kind: "deploy".to_string(),
                name: "deploy/existing".to_string(),
                runbook_hash: runbook_hash.clone(),
                cwd: ctx.project_path.clone(),
                vars: HashMap::new(),
                initial_step: "run".to_string(),
                created_at_ms: 1000,
                project: String::new(),
                cron: Some("deployer".to_string()),
            },
        })
        .await
        .unwrap();

    // Verify count_active_cron_jobs sees it
    assert_eq!(
        ctx.runtime.count_active_cron_jobs("deployer", ""),
        1,
        "should count 1 active cron job"
    );

    // Start cron with concurrency=1
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "deployer".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "10m".to_string(),
            target: oj_core::RunTarget::job("deploy"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer — should skip due to concurrency
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("deployer", "") })
        .await
        .unwrap();

    // No JobCreated should be emitted (spawn was skipped)
    let has_new_job = events
        .iter()
        .any(|e| matches!(e, Event::JobCreated { id, .. } if id.as_str() != "existing-job-1"));
    assert!(!has_new_job, "should NOT spawn job when at max concurrency");

    // No CronFired should be emitted (spawn was skipped)
    let has_cron_fired = events.iter().any(|e| matches!(e, Event::CronFired { .. }));
    assert!(!has_cron_fired, "CronFired should NOT be emitted when spawn is skipped");

    // Timer should still be rescheduled
    let scheduler = ctx.runtime.executor.scheduler();
    let sched = scheduler.lock();
    assert!(sched.has_timers(), "timer should be rescheduled after concurrency skip");
}

// ---- Test 18: cron_job_concurrency_respawns_after_complete ----

#[tokio::test]
async fn cron_job_concurrency_respawns_after_complete() {
    let runbook = test_runbook_cron_job(
        "deployer",
        "deploy",
        "interval = \"10m\"\nconcurrency = 1",
        &[
            ("run", "echo deploying", "on_done = { step = \"done\" }"),
            ("done", "echo finished", ""),
        ],
    );
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject a completed (terminal) job with cron_name = Some("deployer")
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobCreated {
                id: JobId::from_string("completed-job-1"),
                kind: "deploy".to_string(),
                name: "deploy/completed".to_string(),
                runbook_hash: runbook_hash.clone(),
                cwd: ctx.project_path.clone(),
                vars: HashMap::new(),
                initial_step: "run".to_string(),
                created_at_ms: 1000,
                project: String::new(),
                cron: Some("deployer".to_string()),
            },
        })
        .await
        .unwrap();

    // Advance it to terminal state
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobAdvanced {
                id: JobId::from_string("completed-job-1"),
                step: "done".to_string(),
            },
        })
        .await
        .unwrap();

    // Verify it doesn't count as active
    assert_eq!(
        ctx.runtime.count_active_cron_jobs("deployer", ""),
        0,
        "completed job should not count as active"
    );

    // Start cron with concurrency=1
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "deployer".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "10m".to_string(),
            target: oj_core::RunTarget::job("deploy"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer — should succeed since previous job is completed
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("deployer", "") })
        .await
        .unwrap();

    // JobCreated should be emitted (spawn succeeded)
    let has_job = events.iter().any(|e| matches!(e, Event::JobCreated { .. }));
    assert!(has_job, "should spawn job when previous run is completed");

    // CronFired should be emitted
    let has_cron_fired =
        events.iter().any(|e| matches!(e, Event::CronFired { cron, .. } if cron == "deployer"));
    assert!(has_cron_fired, "CronFired should be emitted after successful spawn");
}

// ---- Test 19: cron_job_concurrency_default_singleton ----

#[tokio::test]
async fn cron_job_concurrency_default_singleton() {
    let runbook = test_runbook_cron_job(
        "deployer",
        "deploy",
        "interval = \"10m\"",
        &[
            ("run", "echo deploying", "on_done = { step = \"done\" }"),
            ("done", "echo finished", ""),
        ],
    );
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject an active job with matching cron_name
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobCreated {
                id: JobId::from_string("active-job-1"),
                kind: "deploy".to_string(),
                name: "deploy/active".to_string(),
                runbook_hash: runbook_hash.clone(),
                cwd: ctx.project_path.clone(),
                vars: HashMap::new(),
                initial_step: "run".to_string(),
                created_at_ms: 1000,
                project: String::new(),
                cron: Some("deployer".to_string()),
            },
        })
        .await
        .unwrap();

    // Start cron (no concurrency field = default 1)
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "deployer".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "10m".to_string(),
            target: oj_core::RunTarget::job("deploy"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("deployer", "") })
        .await
        .unwrap();

    // Spawn should be skipped (default concurrency=1 makes it singleton)
    let has_new_job = events
        .iter()
        .any(|e| matches!(e, Event::JobCreated { id, .. } if id.as_str() != "active-job-1"));
    assert!(!has_new_job, "default concurrency=1 should make cron singleton");
}

// ---- Test 20: cron_job_concurrency_allows_multiple ----

#[tokio::test]
async fn cron_job_concurrency_allows_multiple() {
    let runbook = test_runbook_cron_job(
        "deployer",
        "deploy",
        "interval = \"10m\"\nconcurrency = 2",
        &[
            ("run", "echo deploying", "on_done = { step = \"done\" }"),
            ("done", "echo finished", ""),
        ],
    );
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject one active job with matching cron_name
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobCreated {
                id: JobId::from_string("active-job-1"),
                kind: "deploy".to_string(),
                name: "deploy/active".to_string(),
                runbook_hash: runbook_hash.clone(),
                cwd: ctx.project_path.clone(),
                vars: HashMap::new(),
                initial_step: "run".to_string(),
                created_at_ms: 1000,
                project: String::new(),
                cron: Some("deployer".to_string()),
            },
        })
        .await
        .unwrap();

    // Start cron with concurrency=2
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "deployer".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "10m".to_string(),
            target: oj_core::RunTarget::job("deploy"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("deployer", "") })
        .await
        .unwrap();

    // JobCreated SHOULD be emitted (1 < 2, room for another)
    let has_new_job = events
        .iter()
        .any(|e| matches!(e, Event::JobCreated { id, .. } if id.as_str() != "active-job-1"));
    assert!(has_new_job, "concurrency=2 should allow second job when only 1 active");

    // CronFired should be emitted
    let has_cron_fired = events.iter().any(|e| matches!(e, Event::CronFired { .. }));
    assert!(has_cron_fired, "CronFired should be emitted for successful spawn");
}
