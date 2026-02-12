// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::super::test_helpers::*;

#[tokio::test]
async fn reconcile_state_resumes_running_workers() {
    // Workers with status "running" should be re-emitted as RunbookLoaded +
    // WorkerStarted events during reconciliation so the runtime recreates
    // their in-memory state and triggers an initial queue poll.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    // Build state with a running worker and a stopped worker
    let mut test_state = MaterializedState::default();
    // Populate runbooks via apply_event (same path as WAL replay)
    test_state.apply_event(&Event::RunbookLoaded {
        hash: "abc123".to_string(),
        version: 1,
        runbook: serde_json::json!({"test": true}),
    });
    test_state.workers.insert(
        "myns/running-worker".to_string(),
        WorkerRecord {
            name: "running-worker".to_string(),
            project: "myns".to_string(),
            project_path: dir_path.clone(),
            runbook_hash: "abc123".to_string(),
            status: "running".to_string(),
            active: vec![],
            queue: "tasks".to_string(),
            concurrency: 2,
            owners: HashMap::new(),
        },
    );
    test_state.workers.insert(
        "myns/stopped-worker".to_string(),
        WorkerRecord {
            name: "stopped-worker".to_string(),
            project: "myns".to_string(),
            project_path: dir_path.clone(),
            runbook_hash: "def456".to_string(),
            status: "stopped".to_string(),
            active: vec![],
            queue: "other".to_string(),
            concurrency: 1,
            owners: HashMap::new(),
        },
    );

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    // Should have emitted RunbookLoaded before WorkerStarted
    let runbook_loaded_events: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::RunbookLoaded { .. })).collect();
    assert_eq!(
        runbook_loaded_events.len(),
        1,
        "should emit RunbookLoaded for the running worker's hash, got: {:?}",
        runbook_loaded_events
    );
    match &runbook_loaded_events[0] {
        Event::RunbookLoaded { hash, .. } => {
            assert_eq!(hash, "abc123");
        }
        _ => unreachable!(),
    }

    // RunbookLoaded must come before WorkerStarted
    let runbook_pos = events
        .iter()
        .position(|e| matches!(e, Event::RunbookLoaded { hash, .. } if hash == "abc123"))
        .expect("RunbookLoaded should be in events");
    let worker_pos = events
        .iter()
        .position(
            |e| matches!(e, Event::WorkerStarted { worker, .. } if worker == "running-worker"),
        )
        .expect("WorkerStarted should be in events");
    assert!(
        runbook_pos < worker_pos,
        "RunbookLoaded (pos={}) must come before WorkerStarted (pos={})",
        runbook_pos,
        worker_pos
    );

    // Should have emitted exactly one WorkerStarted for the running worker
    let worker_started_events: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::WorkerStarted { .. })).collect();
    assert_eq!(
        worker_started_events.len(),
        1,
        "should emit WorkerStarted for the one running worker, got: {:?}",
        worker_started_events
    );

    // Verify the event has the right fields
    match &worker_started_events[0] {
        Event::WorkerStarted {
            worker,
            project_path,
            runbook_hash,
            queue,
            concurrency,
            project,
        } => {
            assert_eq!(worker, "running-worker");
            assert_eq!(*project_path, dir_path);
            assert_eq!(runbook_hash, "abc123");
            assert_eq!(queue, "tasks");
            assert_eq!(*concurrency, 2);
            assert_eq!(project, "myns");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn reconcile_deduplicates_runbook_loaded_for_shared_hash() {
    // When multiple workers share the same runbook hash, only one
    // RunbookLoaded event should be emitted (dedup by hash).
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    let mut test_state = MaterializedState::default();
    test_state.apply_event(&Event::RunbookLoaded {
        hash: "shared-hash".to_string(),
        version: 1,
        runbook: serde_json::json!({"shared": true}),
    });
    // Two workers sharing the same runbook hash
    for name in &["worker-a", "worker-b"] {
        test_state.workers.insert(
            format!("ns/{}", name),
            WorkerRecord {
                name: name.to_string(),
                project: "ns".to_string(),
                project_path: dir_path.clone(),
                runbook_hash: "shared-hash".to_string(),
                status: "running".to_string(),
                active: vec![],
                queue: "q".to_string(),
                concurrency: 1,
                owners: HashMap::new(),
            },
        );
    }

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    let runbook_loaded: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::RunbookLoaded { .. })).collect();
    assert_eq!(
        runbook_loaded.len(),
        1,
        "should emit exactly one RunbookLoaded for shared hash, got {}",
        runbook_loaded.len()
    );

    let worker_started: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::WorkerStarted { .. })).collect();
    assert_eq!(worker_started.len(), 2, "should emit WorkerStarted for both workers");
}

#[tokio::test]
async fn reconcile_job_dead_session_uses_step_history_agent_id() {
    // When a job's coop session is dead, reconciliation should emit
    // AgentGone with the agent_id from step_history (a UUID), not a
    // fabricated "{job_id}-{step}" string.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    let agent_uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    let mut test_state = MaterializedState::default();
    test_state.jobs.insert("job-1".to_string(), make_job_with_agent("job-1", "build", agent_uuid));

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    // Should emit AgentGone with the UUID from step_history
    let gone_events: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::AgentGone { .. })).collect();
    assert_eq!(gone_events.len(), 1, "should emit exactly one AgentGone event");

    match &gone_events[0] {
        Event::AgentGone { id, .. } => {
            assert_eq!(
                id.as_str(),
                agent_uuid,
                "AgentGone must use UUID from step_history, not job_id-step"
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn reconcile_job_no_agent_id_in_step_history_emits_job_failed() {
    // When a job has no agent_id in step_history (e.g., shell step
    // or crashed before agent was recorded), reconciliation should
    // emit JobAdvanced{step:"failed"} to terminate the zombie job.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    let mut job = make_job_with_agent("job-2", "work", "any");
    // Clear agent_id from step_history
    job.step_history[0].agent_id = None;

    let mut test_state = MaterializedState::default();
    test_state.jobs.insert("job-2".to_string(), job);

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    // Should emit JobAdvanced{step:"failed"} instead of agent events
    let advanced_events: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::JobAdvanced { .. })).collect();
    assert_eq!(
        advanced_events.len(),
        1,
        "should emit exactly one JobAdvanced event, got: {:?}",
        advanced_events
    );
    match &advanced_events[0] {
        Event::JobAdvanced { id, step } => {
            assert_eq!(id, &JobId::new("job-2"));
            assert_eq!(step, "failed");
        }
        _ => unreachable!(),
    }

    // Should not emit any agent events for this job
    let agent_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::AgentGone { .. } | Event::AgentExited { .. }))
        .collect();
    assert!(
        agent_events.is_empty(),
        "should not emit agent events when step_history has no agent_id, got: {:?}",
        agent_events
    );
}

#[tokio::test]
async fn reconcile_job_no_agent_id_emits_job_failed() {
    // When a job has no agent_id in step_history (daemon crashed before
    // agent was recorded), reconciliation should emit JobAdvanced{step:"failed"}.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    // Job with no agent_id in step_history
    let job = Job::builder().id("job-nosess").step("work").build();

    let mut test_state = MaterializedState::default();
    test_state.jobs.insert("job-nosess".to_string(), job);

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    let advanced_events: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::JobAdvanced { .. })).collect();
    assert_eq!(advanced_events.len(), 1, "should emit exactly one JobAdvanced event");
    match &advanced_events[0] {
        Event::JobAdvanced { id, step } => {
            assert_eq!(id, &JobId::new("job-nosess"));
            assert_eq!(step, "failed");
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn reconcile_job_waiting_no_agent_is_skipped() {
    // A Waiting job with no agent should be skipped (already escalated).
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    let job = Job::builder()
        .id("job-waiting")
        .step("work")
        .step_status(StepStatus::Waiting(Some("escalated".to_string())))
        .build();

    let mut test_state = MaterializedState::default();
    test_state.jobs.insert("job-waiting".to_string(), job);

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    // Should not emit any events for a Waiting job
    let job_events: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::JobAdvanced { .. } | Event::AgentGone { .. } | Event::AgentExited { .. }
            )
        })
        .collect();
    assert!(job_events.is_empty(), "should not emit events for Waiting job, got: {:?}", job_events);
}

#[tokio::test]
async fn reconcile_crew_dead_session_emits_gone_with_correct_id() {
    // When an crew's coop session is dead, reconciliation should
    // emit AgentGone with the agent_id from the crew record.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    let agent_uuid = "deadbeef-1234-5678-9abc-def012345678";
    let mut test_state = MaterializedState::default();
    test_state.crew.insert(
        "run-1".to_string(),
        Crew {
            id: "run-1".to_string(),
            agent_name: "test-agent".to_string(),
            command_name: "do-work".to_string(),
            project: "proj".to_string(),
            cwd: dir_path.clone(),
            runbook_hash: "hash123".to_string(),
            status: CrewStatus::Running,
            agent_id: Some(agent_uuid.to_string()),
            error: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
            actions: Default::default(),
            vars: HashMap::new(),
            last_nudge_at: None,
        },
    );

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    // Should emit AgentGone with the correct UUID
    let gone_events: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::AgentGone { .. })).collect();
    assert_eq!(gone_events.len(), 1, "should emit exactly one AgentGone event");
    match &gone_events[0] {
        Event::AgentGone { id, .. } => {
            assert_eq!(id.as_str(), agent_uuid);
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn reconcile_crew_no_agent_id_marks_failed_directly() {
    // When an crew has no agent_id (daemon crashed before
    // CrewStarted was persisted), reconciliation should directly
    // emit CrewUpdated(Failed) instead of trying to route
    // through AgentExited/AgentGone events that would be dropped.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    let mut test_state = MaterializedState::default();
    test_state.crew.insert(
        "run-2".to_string(),
        Crew {
            id: "run-2".to_string(),
            agent_name: "test-agent".to_string(),
            command_name: "do-work".to_string(),
            project: "proj".to_string(),
            cwd: dir_path.clone(),
            runbook_hash: "hash123".to_string(),
            status: CrewStatus::Starting,
            agent_id: None, // No agent_id yet
            error: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
            actions: Default::default(),
            vars: HashMap::new(),
            last_nudge_at: None,
        },
    );

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    // Should emit CrewUpdated(Failed) directly
    let status_events: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::CrewUpdated { .. })).collect();
    assert_eq!(status_events.len(), 1, "should emit exactly one CrewUpdated event");
    match &status_events[0] {
        Event::CrewUpdated { id, status, reason } => {
            assert_eq!(id, &CrewId::new("run-2"));
            assert_eq!(*status, CrewStatus::Failed);
            assert!(
                reason.as_ref().unwrap().contains("no agent_id"),
                "reason should mention missing agent_id, got: {:?}",
                reason
            );
        }
        _ => unreachable!(),
    }

    // Should NOT emit AgentGone or AgentExited
    let agent_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, Event::AgentGone { .. } | Event::AgentExited { .. }))
        .collect();
    assert!(agent_events.is_empty(), "should not emit AgentGone/AgentExited when agent_id is None");
}

#[tokio::test]
async fn reconcile_crew_dead_agent_emits_gone() {
    // When an crew has an agent_id but the agent is not alive,
    // reconciliation checks coop liveness by agent_id. Since coop isn't
    // running, it emits AgentGone.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_owned();
    let runtime = setup_reconcile_runtime(&dir_path);

    let mut test_state = MaterializedState::default();
    test_state.crew.insert(
        "run-3".to_string(),
        Crew {
            id: "run-3".to_string(),
            agent_name: "test-agent".to_string(),
            command_name: "do-work".to_string(),
            project: "proj".to_string(),
            cwd: dir_path.clone(),
            runbook_hash: "hash123".to_string(),
            status: CrewStatus::Running,
            agent_id: Some("some-uuid".to_string()),
            error: None,
            created_at_ms: 1000,
            updated_at_ms: 2000,
            actions: Default::default(),
            vars: HashMap::new(),
            last_nudge_at: None,
        },
    );

    let events = run_reconcile(&runtime, test_state, dir_path.clone()).await;

    let gone_events: Vec<_> =
        events.iter().filter(|e| matches!(e, Event::AgentGone { .. })).collect();
    assert_eq!(gone_events.len(), 1, "should emit AgentGone when coop is not alive");
    match &gone_events[0] {
        Event::AgentGone { id, .. } => {
            assert_eq!(id.as_str(), "some-uuid");
        }
        _ => unreachable!(),
    }
}
