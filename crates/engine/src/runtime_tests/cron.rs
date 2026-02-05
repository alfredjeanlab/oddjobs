// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron-related runtime tests

use super::*;
use crate::runtime::handlers::cron::{CronRunTarget, CronStatus};
use oj_core::AgentRunStatus;

const CRON_RUNBOOK: &str = r#"
[cron.janitor]
interval = "30m"
run = { job = "cleanup" }

[job.cleanup]

[[job.cleanup.step]]
name = "prune"
run = "echo pruning"
on_done = "done"

[[job.cleanup.step]]
name = "done"
run = "echo finished"
"#;

/// Helper: parse the CRON_RUNBOOK and return (runbook_json, runbook_hash).
fn hash_cron_runbook() -> (serde_json::Value, String) {
    let runbook = oj_runbook::parse_runbook(CRON_RUNBOOK).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let runbook_hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };
    (runbook_json, runbook_hash)
}

/// Helper: emit RunbookLoaded to populate the engine's in-process cache.
async fn load_runbook(ctx: &TestContext, runbook_json: &serde_json::Value, runbook_hash: &str) {
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
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    // Populate runbook cache
    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronOnce event
    let job_id = JobId::new("cron-pipe-1");
    let events = ctx
        .runtime
        .handle_event(Event::CronOnce {
            cron_name: "janitor".to_string(),
            job_id: job_id.clone(),
            job_name: "cleanup/cron-pip".to_string(),
            job_kind: "cleanup".to_string(),
            agent_run_id: None,
            agent_name: None,
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            run_target: "job:cleanup".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Job should be created
    let job = ctx
        .runtime
        .get_job("cron-pipe-1")
        .expect("job should exist");
    assert_eq!(job.kind, "cleanup");
    assert_eq!(job.step, "prune");

    // CronFired tracking event should have been emitted
    let has_cron_fired = events
        .iter()
        .any(|e| matches!(e, Event::CronFired { cron_name, .. } if cron_name == "janitor"));
    assert!(has_cron_fired, "CronFired event should be emitted");
}

// ---- Test 2: cron_start_sets_timer ----

#[tokio::test]
async fn cron_start_sets_timer() {
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronStarted
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "janitor".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "job:cleanup".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Cron state should be Running
    {
        let crons = ctx.runtime.cron_states.lock();
        let state = crons.get("janitor").expect("cron state should exist");
        assert_eq!(state.status, CronStatus::Running);
        assert!(
            matches!(state.run_target, crate::runtime::handlers::cron::CronRunTarget::Job(ref p) if p == "cleanup")
        );
        assert_eq!(state.interval, "30m");
    }

    // Timer should have been set
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    assert!(sched.has_timers(), "timer should be set after CronStarted");

    // Advance clock past 30m and check that cron timer fires
    ctx.clock
        .advance(std::time::Duration::from_secs(30 * 60 + 1));
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
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "janitor".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "job:cleanup".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Stop cron
    ctx.runtime
        .handle_event(Event::CronStopped {
            cron_name: "janitor".to_string(),
            namespace: String::new(),
        })
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
    ctx.clock
        .advance(std::time::Duration::from_secs(30 * 60 + 1));
    let fired = sched.fired_timers(ctx.clock.now());
    let cron_timers: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } if id.as_str().starts_with("cron:") => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        cron_timers.is_empty(),
        "no cron timers should fire after stop: {:?}",
        cron_timers
    );
}

// ---- Test 4: cron_timer_fired_creates_job ----

#[tokio::test]
async fn cron_timer_fired_creates_job() {
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron (registers state + sets timer)
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "janitor".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "job:cleanup".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Simulate timer firing via TimerStart event
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("janitor", ""),
        })
        .await
        .unwrap();

    // Job should have been created
    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1, "one job should be created");

    let job = jobs.values().next().unwrap();
    assert_eq!(job.kind, "cleanup");
    assert_eq!(job.step, "prune");

    // CronFired event should be in result
    let has_cron_fired = events
        .iter()
        .any(|e| matches!(e, Event::CronFired { cron_name, .. } if cron_name == "janitor"));
    assert!(has_cron_fired, "CronFired event should be emitted");
}

// ---- Test 5: cron_timer_fired_reloads_runbook ----

#[tokio::test]
async fn cron_timer_fired_reloads_runbook() {
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "janitor".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "job:cleanup".to_string(),
            namespace: String::new(),
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
on_done = "done"

[[job.cleanup.step]]
name = "done"
run = "echo finished v2"
"#;
    let runbook_path = ctx.project_root.join(".oj/runbooks/test.toml");
    std::fs::write(&runbook_path, modified_runbook).unwrap();

    // Fire timer — should reload runbook from disk
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("janitor", ""),
        })
        .await
        .unwrap();

    // Verify runbook hash was updated in cron state
    let new_hash = {
        let crons = ctx.runtime.cron_states.lock();
        crons.get("janitor").unwrap().runbook_hash.clone()
    };
    assert_ne!(
        new_hash, runbook_hash,
        "runbook hash should change after modification"
    );
}

// ---- Test 6: cron_once_job_steps_execute ----

#[tokio::test]
async fn cron_once_job_steps_execute() {
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronOnce
    let job_id = JobId::new("cron-exec-1");
    ctx.runtime
        .handle_event(Event::CronOnce {
            cron_name: "janitor".to_string(),
            job_id: job_id.clone(),
            job_name: "cleanup/cron-exe".to_string(),
            job_kind: "cleanup".to_string(),
            agent_run_id: None,
            agent_name: None,
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            run_target: "job:cleanup".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Job should be at first step
    let job = ctx.runtime.get_job("cron-exec-1").unwrap();
    assert_eq!(job.step, "prune");

    // Simulate shell completion of first step
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: job_id.clone(),
            step: "prune".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Job should advance to "done" step
    let job = ctx.runtime.get_job("cron-exec-1").unwrap();
    assert_eq!(job.step, "done");

    // Complete the "done" step
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: job_id.clone(),
            step: "done".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Job should be terminal
    let job = ctx.runtime.get_job("cron-exec-1").unwrap();
    assert!(
        job.is_terminal(),
        "job should be terminal after all steps complete"
    );
}

// ---- Test 7: cron_timer_fired_reschedules_timer ----

#[tokio::test]
async fn cron_timer_fired_reschedules_timer() {
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "janitor".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "job:cleanup".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer (simulates the first interval expiring)
    ctx.runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("janitor", ""),
        })
        .await
        .unwrap();

    // After firing, the handler should reschedule the timer for the next interval.
    // Verify the scheduler has a new timer pending.
    let scheduler = ctx.runtime.executor.scheduler();
    let sched = scheduler.lock();
    assert!(
        sched.has_timers(),
        "timer should be rescheduled after cron fires"
    );
}

// ---- Test 8: cron_timer_fired_when_stopped_is_noop ----

#[tokio::test]
async fn cron_timer_fired_when_stopped_is_noop() {
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start and immediately stop the cron
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "janitor".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "job:cleanup".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    ctx.runtime
        .handle_event(Event::CronStopped {
            cron_name: "janitor".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Simulate a timer firing for the stopped cron (race condition scenario)
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("janitor", ""),
        })
        .await
        .unwrap();

    // No job should be created
    let jobs = ctx.runtime.jobs();
    assert!(
        jobs.is_empty(),
        "no job should be created for a stopped cron"
    );

    // No CronFired event should be emitted
    let has_cron_fired = events.iter().any(|e| matches!(e, Event::CronFired { .. }));
    assert!(
        !has_cron_fired,
        "no CronFired event should be emitted for stopped cron"
    );
}

// ---- Test 9: cron_start_with_namespace ----

#[tokio::test]
async fn cron_start_with_namespace() {
    let ctx = setup_with_runbook(CRON_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_cron_runbook();

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron with a namespace
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "janitor".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "job:cleanup".to_string(),
            namespace: "myproject".to_string(),
        })
        .await
        .unwrap();

    // Cron state should include namespace
    {
        let crons = ctx.runtime.cron_states.lock();
        let state = crons.get("janitor").expect("cron state should exist");
        assert_eq!(state.namespace, "myproject");
    }

    // Timer ID should include namespace prefix
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock
        .advance(std::time::Duration::from_secs(30 * 60 + 1));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        timer_ids.iter().any(|id| *id == "cron:myproject/janitor"),
        "timer ID should include namespace: {:?}",
        timer_ids
    );
}

// ===========================================================================
// Agent-targeted cron tests
// ===========================================================================

const CRON_AGENT_RUNBOOK: &str = r#"
[cron.health_check]
interval = "30m"
run = { agent = "doctor" }

[agent.doctor]
run = "claude --print"
prompt = "Run diagnostics"
"#;

const CRON_AGENT_MAX_CONC_RUNBOOK: &str = r#"
[cron.health_check]
interval = "30m"
run = { agent = "doctor" }

[agent.doctor]
max_concurrency = 1
run = "claude --print"
prompt = "Run diagnostics"
"#;

fn hash_runbook(content: &str) -> (serde_json::Value, String) {
    let runbook = oj_runbook::parse_runbook(content).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let runbook_hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };
    (runbook_json, runbook_hash)
}

// ---- Test 10: cron_once_agent ----

#[tokio::test]
async fn cron_once_agent() {
    let ctx = setup_with_runbook(CRON_AGENT_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_AGENT_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronOnce targeting an agent
    let events = ctx
        .runtime
        .handle_event(Event::CronOnce {
            cron_name: "health_check".to_string(),
            job_id: JobId::default(),
            job_name: String::new(),
            job_kind: String::new(),
            agent_run_id: Some("ar-once-1".to_string()),
            agent_name: Some("doctor".to_string()),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            run_target: "agent:doctor".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // CronFired event should be emitted with agent_run_id
    let cron_fired = events
        .iter()
        .find(|e| matches!(e, Event::CronFired { cron_name, .. } if cron_name == "health_check"));
    assert!(cron_fired.is_some(), "CronFired event should be emitted");
    if let Some(Event::CronFired { agent_run_id, .. }) = cron_fired {
        assert!(agent_run_id.is_some(), "agent_run_id should be set");
    }

    // AgentRunCreated event should be emitted
    let has_agent_run_created = events.iter().any(|e| {
        matches!(e, Event::AgentRunCreated { agent_name, command_name, .. }
            if agent_name == "doctor" && command_name == "cron:health_check")
    });
    assert!(
        has_agent_run_created,
        "AgentRunCreated should be emitted for cron-once agent"
    );
}

// ---- Test 11: cron_start_agent_sets_timer ----

#[tokio::test]
async fn cron_start_agent_sets_timer() {
    let ctx = setup_with_runbook(CRON_AGENT_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_AGENT_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronStarted with agent target
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "health_check".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "agent:doctor".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Cron state should target an agent
    {
        let crons = ctx.runtime.cron_states.lock();
        let state = crons.get("health_check").expect("cron state should exist");
        assert_eq!(state.status, CronStatus::Running);
        assert!(
            matches!(state.run_target, CronRunTarget::Agent(ref a) if a == "doctor"),
            "run_target should be Agent(doctor)"
        );
    }

    // Timer should be set
    let scheduler = ctx.runtime.executor.scheduler();
    let sched = scheduler.lock();
    assert!(sched.has_timers(), "timer should be set");
}

// ---- Test 12: cron_timer_fires_agent ----

#[tokio::test]
async fn cron_timer_fires_agent() {
    let ctx = setup_with_runbook(CRON_AGENT_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_AGENT_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron targeting agent
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "health_check".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "agent:doctor".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("health_check", ""),
        })
        .await
        .unwrap();

    // AgentRunCreated should be emitted
    let has_agent_run = events.iter().any(|e| {
        matches!(e, Event::AgentRunCreated { agent_name, command_name, .. }
            if agent_name == "doctor" && command_name == "cron:health_check")
    });
    assert!(
        has_agent_run,
        "AgentRunCreated should be emitted on cron timer fire"
    );

    // CronFired should be emitted with agent_run_id
    let has_cron_fired = events.iter().any(|e| {
        matches!(e, Event::CronFired { cron_name, agent_run_id, .. }
            if cron_name == "health_check" && agent_run_id.is_some())
    });
    assert!(
        has_cron_fired,
        "CronFired should be emitted with agent_run_id"
    );

    // No jobs should be created
    let jobs = ctx.runtime.jobs();
    assert!(jobs.is_empty(), "no jobs should be created for agent cron");
}

// ---- Test 13: cron_agent_concurrency_skip ----

#[tokio::test]
async fn cron_agent_concurrency_skip() {
    let ctx = setup_with_runbook(CRON_AGENT_MAX_CONC_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_AGENT_MAX_CONC_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject a running agent into state (simulating an existing running instance)
    // Use executor.execute(Effect::Emit) so state.apply_event() is called
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::AgentRunCreated {
                id: oj_core::AgentRunId::new("existing-run-1"),
                agent_name: "doctor".to_string(),
                command_name: "cron:health_check".to_string(),
                namespace: String::new(),
                cwd: ctx.project_root.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_epoch_ms: 1000,
            },
        })
        .await
        .unwrap();

    // Verify count_running_agents sees it
    assert_eq!(
        ctx.runtime.count_running_agents("doctor", ""),
        1,
        "should count 1 running agent"
    );

    // Start cron targeting agent with max_concurrency=1
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "health_check".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "agent:doctor".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer — should skip due to max_concurrency
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("health_check", ""),
        })
        .await
        .unwrap();

    // No AgentRunCreated should be emitted (spawn was skipped)
    let has_new_agent = events
        .iter()
        .any(|e| matches!(e, Event::AgentRunCreated { agent_name, .. } if agent_name == "doctor"));
    assert!(
        !has_new_agent,
        "should NOT spawn agent when at max concurrency"
    );

    // No CronFired should be emitted (spawn was skipped)
    let has_cron_fired = events.iter().any(|e| matches!(e, Event::CronFired { .. }));
    assert!(
        !has_cron_fired,
        "CronFired should NOT be emitted when spawn is skipped"
    );

    // Timer should still be rescheduled (so it tries again next interval)
    let scheduler = ctx.runtime.executor.scheduler();
    let sched = scheduler.lock();
    assert!(
        sched.has_timers(),
        "timer should be rescheduled after concurrency skip"
    );
}

// ---- Test 14: cron_agent_concurrency_respawns_after_complete ----

#[tokio::test]
async fn cron_agent_concurrency_respawns_after_complete() {
    let ctx = setup_with_runbook(CRON_AGENT_MAX_CONC_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_AGENT_MAX_CONC_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject a completed agent run (should NOT count against concurrency)
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::AgentRunCreated {
                id: oj_core::AgentRunId::new("completed-run-1"),
                agent_name: "doctor".to_string(),
                command_name: "cron:health_check".to_string(),
                namespace: String::new(),
                cwd: ctx.project_root.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_epoch_ms: 1000,
            },
        })
        .await
        .unwrap();

    // Mark it as completed
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::AgentRunStatusChanged {
                id: oj_core::AgentRunId::new("completed-run-1"),
                status: AgentRunStatus::Completed,
                reason: None,
            },
        })
        .await
        .unwrap();

    // Should not count as running
    assert_eq!(
        ctx.runtime.count_running_agents("doctor", ""),
        0,
        "completed agent should not count as running"
    );

    // Start cron targeting agent with max_concurrency=1
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "health_check".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            run_target: "agent:doctor".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer — should succeed since previous run is completed
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("health_check", ""),
        })
        .await
        .unwrap();

    // AgentRunCreated should be emitted (spawn succeeded)
    let has_agent_run = events
        .iter()
        .any(|e| matches!(e, Event::AgentRunCreated { agent_name, .. } if agent_name == "doctor"));
    assert!(
        has_agent_run,
        "should spawn agent when previous run is completed"
    );

    // CronFired should be emitted
    let has_cron_fired = events
        .iter()
        .any(|e| matches!(e, Event::CronFired { cron_name, .. } if cron_name == "health_check"));
    assert!(
        has_cron_fired,
        "CronFired should be emitted after successful spawn"
    );
}

// ---- Test 15: count_running_agents_standalone ----

#[tokio::test]
async fn count_running_agents_standalone() {
    let ctx = setup_with_runbook(CRON_AGENT_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_AGENT_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Initially no running agents
    assert_eq!(ctx.runtime.count_running_agents("doctor", ""), 0);

    // Add a starting agent
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::AgentRunCreated {
                id: oj_core::AgentRunId::new("run-1"),
                agent_name: "doctor".to_string(),
                command_name: "test".to_string(),
                namespace: String::new(),
                cwd: ctx.project_root.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_epoch_ms: 1000,
            },
        })
        .await
        .unwrap();

    assert_eq!(
        ctx.runtime.count_running_agents("doctor", ""),
        1,
        "Starting agent should count"
    );

    // Add another agent
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::AgentRunCreated {
                id: oj_core::AgentRunId::new("run-2"),
                agent_name: "doctor".to_string(),
                command_name: "test".to_string(),
                namespace: String::new(),
                cwd: ctx.project_root.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_epoch_ms: 2000,
            },
        })
        .await
        .unwrap();

    assert_eq!(
        ctx.runtime.count_running_agents("doctor", ""),
        2,
        "Two non-terminal agents"
    );

    // Complete one
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::AgentRunStatusChanged {
                id: oj_core::AgentRunId::new("run-1"),
                status: AgentRunStatus::Completed,
                reason: None,
            },
        })
        .await
        .unwrap();

    assert_eq!(
        ctx.runtime.count_running_agents("doctor", ""),
        1,
        "Completed agent should not count"
    );

    // Fail the other
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::AgentRunStatusChanged {
                id: oj_core::AgentRunId::new("run-2"),
                status: AgentRunStatus::Failed,
                reason: Some("crashed".to_string()),
            },
        })
        .await
        .unwrap();

    assert_eq!(
        ctx.runtime.count_running_agents("doctor", ""),
        0,
        "Failed agent should not count"
    );
}

// ---- Test 16: count_running_agents_namespace_isolation ----

#[tokio::test]
async fn count_running_agents_namespace_isolation() {
    let ctx = setup_with_runbook(CRON_AGENT_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_AGENT_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Add agent in namespace "ns-a"
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::AgentRunCreated {
                id: oj_core::AgentRunId::new("run-ns-a"),
                agent_name: "doctor".to_string(),
                command_name: "test".to_string(),
                namespace: "ns-a".to_string(),
                cwd: ctx.project_root.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_epoch_ms: 1000,
            },
        })
        .await
        .unwrap();

    // Count in namespace "ns-a" should be 1
    assert_eq!(ctx.runtime.count_running_agents("doctor", "ns-a"), 1);

    // Count in empty namespace should be 0 (different namespace)
    assert_eq!(ctx.runtime.count_running_agents("doctor", ""), 0);

    // Count in namespace "ns-b" should be 0
    assert_eq!(ctx.runtime.count_running_agents("doctor", "ns-b"), 0);
}

// ===========================================================================
// Job-targeted cron concurrency tests
// ===========================================================================

const CRON_JOB_CONC_RUNBOOK: &str = r#"
[cron.deployer]
interval = "10m"
concurrency = 1
run = { job = "deploy" }

[job.deploy]
[[job.deploy.step]]
name = "run"
run = "echo deploying"
on_done = "done"

[[job.deploy.step]]
name = "done"
run = "echo finished"
"#;

const CRON_JOB_CONC2_RUNBOOK: &str = r#"
[cron.deployer]
interval = "10m"
concurrency = 2
run = { job = "deploy" }

[job.deploy]
[[job.deploy.step]]
name = "run"
run = "echo deploying"
on_done = "done"

[[job.deploy.step]]
name = "done"
run = "echo finished"
"#;

const CRON_JOB_NO_CONC_RUNBOOK: &str = r#"
[cron.deployer]
interval = "10m"
run = { job = "deploy" }

[job.deploy]
[[job.deploy.step]]
name = "run"
run = "echo deploying"
on_done = "done"

[[job.deploy.step]]
name = "done"
run = "echo finished"
"#;

// ---- Test 17: cron_job_concurrency_skip ----

#[tokio::test]
async fn cron_job_concurrency_skip() {
    let ctx = setup_with_runbook(CRON_JOB_CONC_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_JOB_CONC_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject an active (non-terminal) job with cron_name = Some("deployer")
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobCreated {
                id: JobId::new("existing-pipe-1"),
                kind: "deploy".to_string(),
                name: "deploy/existing".to_string(),
                runbook_hash: runbook_hash.clone(),
                cwd: ctx.project_root.clone(),
                vars: HashMap::new(),
                initial_step: "run".to_string(),
                created_at_epoch_ms: 1000,
                namespace: String::new(),
                cron_name: Some("deployer".to_string()),
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
            cron_name: "deployer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "10m".to_string(),
            run_target: "job:deploy".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer — should skip due to concurrency
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("deployer", ""),
        })
        .await
        .unwrap();

    // No JobCreated should be emitted (spawn was skipped)
    let has_new_job = events
        .iter()
        .any(|e| matches!(e, Event::JobCreated { id, .. } if id.as_str() != "existing-pipe-1"));
    assert!(!has_new_job, "should NOT spawn job when at max concurrency");

    // No CronFired should be emitted (spawn was skipped)
    let has_cron_fired = events.iter().any(|e| matches!(e, Event::CronFired { .. }));
    assert!(
        !has_cron_fired,
        "CronFired should NOT be emitted when spawn is skipped"
    );

    // Timer should still be rescheduled
    let scheduler = ctx.runtime.executor.scheduler();
    let sched = scheduler.lock();
    assert!(
        sched.has_timers(),
        "timer should be rescheduled after concurrency skip"
    );
}

// ---- Test 18: cron_job_concurrency_respawns_after_complete ----

#[tokio::test]
async fn cron_job_concurrency_respawns_after_complete() {
    let ctx = setup_with_runbook(CRON_JOB_CONC_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_JOB_CONC_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject a completed (terminal) job with cron_name = Some("deployer")
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobCreated {
                id: JobId::new("completed-pipe-1"),
                kind: "deploy".to_string(),
                name: "deploy/completed".to_string(),
                runbook_hash: runbook_hash.clone(),
                cwd: ctx.project_root.clone(),
                vars: HashMap::new(),
                initial_step: "run".to_string(),
                created_at_epoch_ms: 1000,
                namespace: String::new(),
                cron_name: Some("deployer".to_string()),
            },
        })
        .await
        .unwrap();

    // Advance it to terminal state
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobAdvanced {
                id: JobId::new("completed-pipe-1"),
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
            cron_name: "deployer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "10m".to_string(),
            run_target: "job:deploy".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer — should succeed since previous job is completed
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("deployer", ""),
        })
        .await
        .unwrap();

    // JobCreated should be emitted (spawn succeeded)
    let has_job = events.iter().any(|e| matches!(e, Event::JobCreated { .. }));
    assert!(has_job, "should spawn job when previous run is completed");

    // CronFired should be emitted
    let has_cron_fired = events
        .iter()
        .any(|e| matches!(e, Event::CronFired { cron_name, .. } if cron_name == "deployer"));
    assert!(
        has_cron_fired,
        "CronFired should be emitted after successful spawn"
    );
}

// ---- Test 19: cron_job_concurrency_default_singleton ----

#[tokio::test]
async fn cron_job_concurrency_default_singleton() {
    let ctx = setup_with_runbook(CRON_JOB_NO_CONC_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_JOB_NO_CONC_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject an active job with matching cron_name
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobCreated {
                id: JobId::new("active-pipe-1"),
                kind: "deploy".to_string(),
                name: "deploy/active".to_string(),
                runbook_hash: runbook_hash.clone(),
                cwd: ctx.project_root.clone(),
                vars: HashMap::new(),
                initial_step: "run".to_string(),
                created_at_epoch_ms: 1000,
                namespace: String::new(),
                cron_name: Some("deployer".to_string()),
            },
        })
        .await
        .unwrap();

    // Start cron (no concurrency field = default 1)
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "deployer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "10m".to_string(),
            run_target: "job:deploy".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("deployer", ""),
        })
        .await
        .unwrap();

    // Spawn should be skipped (default concurrency=1 makes it singleton)
    let has_new_job = events
        .iter()
        .any(|e| matches!(e, Event::JobCreated { id, .. } if id.as_str() != "active-pipe-1"));
    assert!(
        !has_new_job,
        "default concurrency=1 should make cron singleton"
    );
}

// ---- Test 20: cron_job_concurrency_allows_multiple ----

#[tokio::test]
async fn cron_job_concurrency_allows_multiple() {
    let ctx = setup_with_runbook(CRON_JOB_CONC2_RUNBOOK).await;
    let (runbook_json, runbook_hash) = hash_runbook(CRON_JOB_CONC2_RUNBOOK);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject one active job with matching cron_name
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::JobCreated {
                id: JobId::new("active-pipe-1"),
                kind: "deploy".to_string(),
                name: "deploy/active".to_string(),
                runbook_hash: runbook_hash.clone(),
                cwd: ctx.project_root.clone(),
                vars: HashMap::new(),
                initial_step: "run".to_string(),
                created_at_epoch_ms: 1000,
                namespace: String::new(),
                cron_name: Some("deployer".to_string()),
            },
        })
        .await
        .unwrap();

    // Start cron with concurrency=2
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron_name: "deployer".to_string(),
            project_root: ctx.project_root.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "10m".to_string(),
            run_target: "job:deploy".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart {
            id: oj_core::TimerId::cron("deployer", ""),
        })
        .await
        .unwrap();

    // JobCreated SHOULD be emitted (1 < 2, room for another)
    let has_new_job = events
        .iter()
        .any(|e| matches!(e, Event::JobCreated { id, .. } if id.as_str() != "active-pipe-1"));
    assert!(
        has_new_job,
        "concurrency=2 should allow second job when only 1 active"
    );

    // CronFired should be emitted
    let has_cron_fired = events.iter().any(|e| matches!(e, Event::CronFired { .. }));
    assert!(
        has_cron_fired,
        "CronFired should be emitted for successful spawn"
    );
}
