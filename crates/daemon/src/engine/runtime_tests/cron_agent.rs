// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent-targeted cron tests

use super::*;
use crate::engine::runtime::handlers::cron::CronStatus;
use oj_core::{CrewStatus, OwnerId, RunTarget};

use super::cron::load_runbook;

// ---- Test 10: cron_once_agent ----

#[tokio::test]
async fn cron_once_agent() {
    let runbook = test_runbook_cron_agent("interval = \"30m\"", "");
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronOnce targeting an agent
    let events = ctx
        .runtime
        .handle_event(Event::CronOnce {
            cron: "health_check".to_string(),
            owner: oj_core::CrewId::from_string("run-once-1").into(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            target: RunTarget::agent("doctor"),
            project: String::new(),
        })
        .await
        .unwrap();

    // CronFired event should be emitted with crew owner
    let cron_fired = events
        .iter()
        .find(|e| matches!(e, Event::CronFired { cron, .. } if cron == "health_check"));
    assert!(cron_fired.is_some(), "CronFired event should be emitted");
    if let Some(Event::CronFired { owner, .. }) = cron_fired {
        assert!(matches!(owner, OwnerId::Crew(_)), "owner should be Crew");
    }

    // CrewCreated event should be emitted
    let has_crew_created = events.iter().any(|e| {
        matches!(e, Event::CrewCreated { agent, command, .. }
            if agent == "doctor" && command == "cron:health_check")
    });
    assert!(has_crew_created, "CrewCreated should be emitted for cron-once agent");
}

// ---- Test 11: cron_start_agent_sets_timer ----

#[tokio::test]
async fn cron_start_agent_sets_timer() {
    let runbook = test_runbook_cron_agent("interval = \"30m\"", "");
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Emit CronStarted with agent target
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "health_check".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::agent("doctor"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Cron state should target an agent
    {
        let crons = ctx.runtime.cron_states.lock();
        let state = crons.get("health_check").expect("cron state should exist");
        assert_eq!(state.status, CronStatus::Running);
        assert!(
            matches!(state.target, RunTarget::Agent(ref a) if a == "doctor"),
            "target should be Agent(doctor)"
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
    let runbook = test_runbook_cron_agent("interval = \"30m\"", "");
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Start cron targeting agent
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "health_check".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::agent("doctor"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("health_check", "") })
        .await
        .unwrap();

    // CrewCreated should be emitted
    let has_crew = events.iter().any(|e| {
        matches!(e, Event::CrewCreated { agent, command, .. }
            if agent == "doctor" && command == "cron:health_check")
    });
    assert!(has_crew, "CrewCreated should be emitted on cron timer fire");

    // CronFired should be emitted with crew owner
    let has_cron_fired = events.iter().any(|e| {
        matches!(e, Event::CronFired { cron, owner, .. }
            if cron == "health_check" && matches!(owner, OwnerId::Crew(_)))
    });
    assert!(has_cron_fired, "CronFired should be emitted with crew owner");

    // No jobs should be created
    let jobs = ctx.runtime.jobs();
    assert!(jobs.is_empty(), "no jobs should be created for agent cron");
}

// ---- Test 13: cron_agent_concurrency_skip ----

#[tokio::test]
async fn cron_agent_concurrency_skip() {
    let runbook = test_runbook_cron_agent("interval = \"30m\"", "max_concurrency = 1");
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject a running agent into state (simulating an existing running instance)
    // Use executor.execute(Effect::Emit) so state.apply_event() is called
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::CrewCreated {
                id: oj_core::CrewId::from_string("existing-run-1"),
                agent: "doctor".to_string(),
                command: "cron:health_check".to_string(),
                project: String::new(),
                cwd: ctx.project_path.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_ms: 1000,
            },
        })
        .await
        .unwrap();

    // Verify count_running_agents sees it
    assert_eq!(ctx.runtime.count_running_agents("doctor", ""), 1, "should count 1 running agent");

    // Start cron targeting agent with max_concurrency=1
    ctx.runtime
        .handle_event(Event::CronStarted {
            cron: "health_check".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::agent("doctor"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer — should skip due to max_concurrency
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("health_check", "") })
        .await
        .unwrap();

    // No CrewCreated should be emitted (spawn was skipped)
    let has_new_agent =
        events.iter().any(|e| matches!(e, Event::CrewCreated { agent, .. } if agent == "doctor"));
    assert!(!has_new_agent, "should NOT spawn agent when at max concurrency");

    // No CronFired should be emitted (spawn was skipped)
    let has_cron_fired = events.iter().any(|e| matches!(e, Event::CronFired { .. }));
    assert!(!has_cron_fired, "CronFired should NOT be emitted when spawn is skipped");

    // Timer should still be rescheduled (so it tries again next interval)
    let scheduler = ctx.runtime.executor.scheduler();
    let sched = scheduler.lock();
    assert!(sched.has_timers(), "timer should be rescheduled after concurrency skip");
}

// ---- Test 14: cron_agent_concurrency_respawns_after_complete ----

#[tokio::test]
async fn cron_agent_concurrency_respawns_after_complete() {
    let runbook = test_runbook_cron_agent("interval = \"30m\"", "max_concurrency = 1");
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Inject a completed crew (should NOT count against concurrency)
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::CrewCreated {
                id: oj_core::CrewId::from_string("completed-run-1"),
                agent: "doctor".to_string(),
                command: "cron:health_check".to_string(),
                project: String::new(),
                cwd: ctx.project_path.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_ms: 1000,
            },
        })
        .await
        .unwrap();

    // Mark it as completed
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::CrewUpdated {
                id: oj_core::CrewId::from_string("completed-run-1"),
                status: CrewStatus::Completed,
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
            cron: "health_check".to_string(),
            project_path: ctx.project_path.clone(),
            runbook_hash: runbook_hash.clone(),
            interval: "30m".to_string(),
            target: RunTarget::agent("doctor"),
            project: String::new(),
        })
        .await
        .unwrap();

    // Fire the timer — should succeed since previous run is completed
    let events = ctx
        .runtime
        .handle_event(Event::TimerStart { id: oj_core::TimerId::cron("health_check", "") })
        .await
        .unwrap();

    // CrewCreated should be emitted (spawn succeeded)
    let has_crew =
        events.iter().any(|e| matches!(e, Event::CrewCreated { agent, .. } if agent == "doctor"));
    assert!(has_crew, "should spawn agent when previous run is completed");

    // CronFired should be emitted
    let has_cron_fired =
        events.iter().any(|e| matches!(e, Event::CronFired { cron, .. } if cron == "health_check"));
    assert!(has_cron_fired, "CronFired should be emitted after successful spawn");
}

// ---- Test 15: count_running_agents_standalone ----

#[tokio::test]
async fn count_running_agents_standalone() {
    let runbook = test_runbook_cron_agent("interval = \"30m\"", "");
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Initially no running agents
    assert_eq!(ctx.runtime.count_running_agents("doctor", ""), 0);

    // Add a starting agent
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::CrewCreated {
                id: oj_core::CrewId::from_string("run-1"),
                agent: "doctor".to_string(),
                command: "test".to_string(),
                project: String::new(),
                cwd: ctx.project_path.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_ms: 1000,
            },
        })
        .await
        .unwrap();

    assert_eq!(ctx.runtime.count_running_agents("doctor", ""), 1, "Starting agent should count");

    // Add another agent
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::CrewCreated {
                id: oj_core::CrewId::from_string("run-2"),
                agent: "doctor".to_string(),
                command: "test".to_string(),
                project: String::new(),
                cwd: ctx.project_path.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_ms: 2000,
            },
        })
        .await
        .unwrap();

    assert_eq!(ctx.runtime.count_running_agents("doctor", ""), 2, "Two non-terminal agents");

    // Complete one
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::CrewUpdated {
                id: oj_core::CrewId::from_string("run-1"),
                status: CrewStatus::Completed,
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
            event: Event::CrewUpdated {
                id: oj_core::CrewId::from_string("run-2"),
                status: CrewStatus::Failed,
                reason: Some("crashed".to_string()),
            },
        })
        .await
        .unwrap();

    assert_eq!(ctx.runtime.count_running_agents("doctor", ""), 0, "Failed agent should not count");
}

// ---- Test 16: count_running_agents_namespace_isolation ----

#[tokio::test]
async fn count_running_agents_namespace_isolation() {
    let runbook = test_runbook_cron_agent("interval = \"30m\"", "");
    let ctx = setup_with_runbook(&runbook).await;
    let (runbook_json, runbook_hash) = hash_runbook(&runbook);

    load_runbook(&ctx, &runbook_json, &runbook_hash).await;

    // Add agent in project "ns-a"
    ctx.runtime
        .executor
        .execute(oj_core::Effect::Emit {
            event: Event::CrewCreated {
                id: oj_core::CrewId::from_string("run-ns-a"),
                agent: "doctor".to_string(),
                command: "test".to_string(),
                project: "ns-a".to_string(),
                cwd: ctx.project_path.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_ms: 1000,
            },
        })
        .await
        .unwrap();

    // Count in project "ns-a" should be 1
    assert_eq!(ctx.runtime.count_running_agents("doctor", "ns-a"), 1);

    // Count in empty project should be 0 (different project)
    assert_eq!(ctx.runtime.count_running_agents("doctor", ""), 0);

    // Count in project "ns-b" should be 0
    assert_eq!(ctx.runtime.count_running_agents("doctor", "ns-b"), 0);
}
