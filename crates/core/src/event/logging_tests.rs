// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for `Event::log_summary()` — agent, command, job, runbook, session, shell, step, and system events.

use crate::crew::{CrewId, CrewStatus};
use crate::event::*;
use crate::owner::OwnerId;

#[test]
fn log_summary_agent_state_events() {
    let cases = vec![
        (
            Event::AgentWorking {
                id: AgentId::from_string("a1"),
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:working agent=a1",
        ),
        (
            Event::AgentWaiting {
                id: AgentId::from_string("a2"),
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:waiting agent=a2",
        ),
        (
            Event::AgentFailed {
                id: AgentId::from_string("a3"),
                error: AgentError::RateLimited,
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:failed agent=a3",
        ),
        (
            Event::AgentExited {
                id: AgentId::from_string("a4"),
                exit_code: Some(0),
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:exited agent=a4",
        ),
        (
            Event::AgentGone {
                id: AgentId::from_string("a5"),
                owner: OwnerId::Job(JobId::default()),
                exit_code: None,
            },
            "agent:gone agent=a5",
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(event.log_summary(), expected, "failed for {:?}", event);
    }
}

#[test]
fn log_summary_agent_input() {
    let event = Event::AgentInput { id: AgentId::from_string("a1"), input: "hello".to_string() };
    assert_eq!(event.log_summary(), "agent:input agent=a1");
}

#[test]
fn log_summary_agent_idle() {
    let event = Event::AgentIdle { id: AgentId::from_string("a1") };
    assert_eq!(event.log_summary(), "agent:idle agent=a1");
}

#[test]
fn log_summary_agent_prompt() {
    let event = Event::AgentPrompt {
        id: AgentId::from_string("a1"),
        prompt_type: PromptType::Permission,
        questions: None,
        last_message: None,
    };
    assert_eq!(event.log_summary(), "agent:prompt agent=a1 prompt_type=Permission");
}

#[test]
fn log_summary_command_run_no_namespace() {
    let event = Event::CommandRun {
        owner: OwnerId::Job(JobId::from_string("j1")),
        name: "build".to_string(),
        project_path: PathBuf::from("/proj"),
        invoke_dir: PathBuf::from("/proj"),
        command: "build".to_string(),
        args: HashMap::new(),
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "command:run job=j1 cmd=build");
}

#[test]
fn log_summary_command_run_with_namespace() {
    let event = Event::CommandRun {
        owner: OwnerId::Job(JobId::from_string("j1")),
        name: "build".to_string(),
        project_path: PathBuf::from("/proj"),
        invoke_dir: PathBuf::from("/proj"),
        command: "deploy".to_string(),
        args: HashMap::new(),
        project: "myns".to_string(),
    };
    assert_eq!(event.log_summary(), "command:run job=j1 ns=myns cmd=deploy");
}

#[test]
fn log_summary_job_created_no_namespace() {
    let event = Event::JobCreated {
        id: JobId::from_string("j1"),
        kind: "build".to_string(),
        name: "test".to_string(),
        runbook_hash: "abc".to_string(),
        cwd: PathBuf::from("/"),
        vars: HashMap::new(),
        initial_step: "init".to_string(),
        created_at_ms: 0,
        project: String::new(),
        cron: None,
    };
    assert_eq!(event.log_summary(), "job:created id=j1 kind=build name=test");
}

#[test]
fn log_summary_job_created_with_namespace() {
    let event = Event::JobCreated {
        id: JobId::from_string("j1"),
        kind: "build".to_string(),
        name: "test".to_string(),
        runbook_hash: "abc".to_string(),
        cwd: PathBuf::from("/"),
        vars: HashMap::new(),
        initial_step: "init".to_string(),
        created_at_ms: 0,
        project: "prod".to_string(),
        cron: None,
    };
    assert_eq!(event.log_summary(), "job:created id=j1 ns=prod kind=build name=test");
}

#[test]
fn log_summary_job_advanced() {
    let event = Event::JobAdvanced { id: JobId::from_string("j1"), step: "deploy".to_string() };
    assert_eq!(event.log_summary(), "job:advanced id=j1 step=deploy");
}

#[test]
fn log_summary_job_updated() {
    let event = Event::JobUpdated { id: JobId::from_string("j1"), vars: HashMap::new() };
    assert_eq!(event.log_summary(), "job:updated id=j1");
}

#[test]
fn log_summary_job_resume() {
    let event = Event::JobResume {
        id: JobId::from_string("j1"),
        message: None,
        vars: HashMap::new(),
        kill: false,
    };
    assert_eq!(event.log_summary(), "job:resume id=j1");
}

#[test]
fn log_summary_job_cancelling_cancel_deleted() {
    assert_eq!(
        Event::JobCancelling { id: JobId::from_string("j1") }.log_summary(),
        "job:cancelling id=j1"
    );
    assert_eq!(Event::JobCancel { id: JobId::from_string("j2") }.log_summary(), "job:cancel id=j2");
    assert_eq!(
        Event::JobDeleted { id: JobId::from_string("j3") }.log_summary(),
        "job:deleted id=j3"
    );
}

#[test]
fn log_summary_runbook_loaded() {
    let runbook = serde_json::json!({
        "agents": {"builder": {}, "tester": {}},
        "jobs": {"ci": {}}
    });
    let event = Event::RunbookLoaded { hash: "abcdef1234567890".to_string(), version: 3, runbook };
    assert_eq!(event.log_summary(), "runbook:loaded hash=abcdef123456 v=3 agents=2 jobs=1");
}

#[test]
fn log_summary_runbook_loaded_empty() {
    let runbook = serde_json::json!({});
    let event = Event::RunbookLoaded { hash: "short".to_string(), version: 1, runbook };
    assert_eq!(event.log_summary(), "runbook:loaded hash=short v=1 agents=0 jobs=0");
}

#[test]
fn log_summary_agent_spawned_job_owner() {
    let event = Event::AgentSpawned {
        id: AgentId::from_string("a1"),
        owner: JobId::from_string("j1").into(),
        runtime: Default::default(),
        auth_token: None,
    };
    assert_eq!(event.log_summary(), "agent:spawned agent=a1 owner=j1 runtime=Local");
}

#[test]
fn log_summary_agent_spawned_crew_owner() {
    let event = Event::AgentSpawned {
        id: AgentId::from_string("a1"),
        owner: CrewId::from_string("ar1").into(),
        runtime: Default::default(),
        auth_token: None,
    };
    assert_eq!(event.log_summary(), "agent:spawned agent=a1 owner=ar1 runtime=Local");
}

#[test]
fn log_summary_shell_exited() {
    let event = Event::ShellExited {
        job_id: JobId::from_string("j1"),
        step: "init".to_string(),
        exit_code: 42,
        stdout: None,
        stderr: None,
    };
    assert_eq!(event.log_summary(), "shell:exited job=j1 step=init exit=42");
}

#[test]
fn log_summary_step_events() {
    assert_eq!(
        Event::StepStarted {
            job_id: JobId::from_string("j1"),
            step: "build".to_string(),
            agent_id: None,
            agent_name: None,
        }
        .log_summary(),
        "step:started job=j1 step=build"
    );
    assert_eq!(
        Event::StepWaiting {
            job_id: JobId::from_string("j1"),
            step: "review".to_string(),
            reason: Some("gate failed".to_string()),
            decision_id: None,
        }
        .log_summary(),
        "step:waiting job=j1 step=review"
    );
    assert_eq!(
        Event::StepCompleted { job_id: JobId::from_string("j1"), step: "deploy".to_string() }
            .log_summary(),
        "step:completed job=j1 step=deploy"
    );
    assert_eq!(
        Event::StepFailed {
            job_id: JobId::from_string("j1"),
            step: "test".to_string(),
            error: "oops".to_string(),
        }
        .log_summary(),
        "step:failed job=j1 step=test"
    );
}

#[test]
fn log_summary_shutdown_and_custom() {
    assert_eq!(Event::Shutdown.log_summary(), "system:shutdown");
    assert_eq!(Event::Custom.log_summary(), "custom");
}

#[test]
fn log_summary_timer_start() {
    let event = Event::TimerStart { id: TimerId::from_string("t1") };
    assert_eq!(event.log_summary(), "timer:start id=t1");
}

#[test]
fn log_summary_workspace_events() {
    assert_eq!(
        Event::WorkspaceCreated {
            id: WorkspaceId::from_string("ws1"),
            path: PathBuf::from("/tmp/ws"),
            branch: Some("main".to_string()),
            owner: OwnerId::Job(JobId::from_string("job-1")),
            workspace_type: None,
        }
        .log_summary(),
        "workspace:created id=ws1"
    );
    assert_eq!(
        Event::WorkspaceReady { id: WorkspaceId::from_string("ws1") }.log_summary(),
        "workspace:ready id=ws1"
    );
    assert_eq!(
        Event::WorkspaceFailed {
            id: WorkspaceId::from_string("ws1"),
            reason: "disk full".to_string()
        }
        .log_summary(),
        "workspace:failed id=ws1"
    );
    assert_eq!(
        Event::WorkspaceDeleted { id: WorkspaceId::from_string("ws1") }.log_summary(),
        "workspace:deleted id=ws1"
    );
    assert_eq!(
        Event::WorkspaceDrop { id: WorkspaceId::from_string("ws1") }.log_summary(),
        "workspace:drop id=ws1"
    );
}

#[test]
fn log_summary_cron_started_stopped() {
    assert_eq!(
        Event::CronStarted {
            cron: "nightly".to_string(),
            project_path: PathBuf::from("/proj"),
            runbook_hash: "abc".to_string(),
            interval: "1h".to_string(),
            target: crate::RunTarget::job("build"),
            project: String::new(),
        }
        .log_summary(),
        "cron:started cron=nightly"
    );
    assert_eq!(
        Event::CronStopped { cron: "nightly".to_string(), project: String::new() }.log_summary(),
        "cron:stopped cron=nightly"
    );
}

#[test]
fn log_summary_cron_once_job_target() {
    let event = Event::CronOnce {
        cron: "nightly".to_string(),
        owner: JobId::from_string("j1").into(),
        project_path: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        target: crate::RunTarget::job("build"),
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:once cron=nightly job=build");
}

#[test]
fn log_summary_cron_once_agent_target() {
    let event = Event::CronOnce {
        cron: "nightly".to_string(),
        owner: CrewId::from_string("ar1").into(),
        project_path: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        target: crate::RunTarget::agent("builder"),
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:once cron=nightly agent=builder");
}

#[test]
fn log_summary_cron_fired_job() {
    let event = Event::CronFired {
        cron: "nightly".to_string(),
        owner: JobId::from_string("j1").into(),
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:fired cron=nightly job=j1");
}

#[test]
fn log_summary_cron_fired_crew() {
    let event = Event::CronFired {
        cron: "nightly".to_string(),
        owner: CrewId::from_string("ar1").into(),
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:fired cron=nightly crew=ar1");
}

#[test]
fn log_summary_cron_deleted_no_namespace() {
    let event = Event::CronDeleted { cron: "nightly".to_string(), project: String::new() };
    assert_eq!(event.log_summary(), "cron:deleted cron=nightly");
}

#[test]
fn log_summary_cron_deleted_with_namespace() {
    let event = Event::CronDeleted { cron: "nightly".to_string(), project: "prod".to_string() };
    assert_eq!(event.log_summary(), "cron:deleted cron=nightly ns=prod");
}

#[test]
fn log_summary_worker_events() {
    assert_eq!(
        Event::WorkerStarted {
            worker: "fixer".to_string(),
            project_path: PathBuf::from("/proj"),
            runbook_hash: "abc".to_string(),
            queue: "bugs".to_string(),
            concurrency: 2,
            project: String::new(),
        }
        .log_summary(),
        "worker:started worker=fixer"
    );
    assert_eq!(
        Event::WorkerWake { worker: "fixer".to_string(), project: String::new() }.log_summary(),
        "worker:wake worker=fixer"
    );
    assert_eq!(
        Event::WorkerStopped { worker: "fixer".to_string(), project: String::new() }.log_summary(),
        "worker:stopped worker=fixer"
    );
}

#[test]
fn log_summary_worker_poll_complete() {
    let event = Event::WorkerPolled {
        worker: "fixer".to_string(),
        project: String::new(),
        items: vec![serde_json::json!({"id": "1"}), serde_json::json!({"id": "2"})],
    };
    assert_eq!(event.log_summary(), "worker:polled worker=fixer items=2");
}

#[test]
fn log_summary_worker_take_complete() {
    let event = Event::WorkerTook {
        worker: "fixer".to_string(),
        project: String::new(),
        item_id: "item-1".to_string(),
        item: serde_json::json!({}),
        exit_code: 0,
        stderr: None,
    };
    assert_eq!(event.log_summary(), "worker:took worker=fixer item=item-1 exit=0");
}

#[test]
fn log_summary_worker_item_dispatched() {
    let event = Event::WorkerDispatched {
        worker: "fixer".to_string(),
        item_id: "item-1".to_string(),
        owner: JobId::from_string("j1").into(),
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "worker:dispatched worker=fixer item=item-1 owner=j1");
}

#[test]
fn log_summary_worker_resized_no_namespace() {
    let event = Event::WorkerResized {
        worker: "fixer".to_string(),
        concurrency: 4,
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "worker:resized worker=fixer concurrency=4");
}

#[test]
fn log_summary_worker_resized_with_namespace() {
    let event = Event::WorkerResized {
        worker: "fixer".to_string(),
        concurrency: 4,
        project: "prod".to_string(),
    };
    assert_eq!(event.log_summary(), "worker:resized worker=fixer ns=prod concurrency=4");
}

#[test]
fn log_summary_worker_deleted_no_namespace() {
    let event = Event::WorkerDeleted { worker: "fixer".to_string(), project: String::new() };
    assert_eq!(event.log_summary(), "worker:deleted worker=fixer");
}

#[test]
fn log_summary_worker_deleted_with_namespace() {
    let event = Event::WorkerDeleted { worker: "fixer".to_string(), project: "prod".to_string() };
    assert_eq!(event.log_summary(), "worker:deleted worker=fixer ns=prod");
}

#[test]
fn log_summary_queue_events() {
    assert_eq!(
        Event::QueuePushed {
            queue: "bugs".to_string(),
            item_id: "i1".to_string(),
            data: HashMap::new(),
            pushed_at_ms: 0,
            project: String::new(),
        }
        .log_summary(),
        "queue:pushed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueTaken {
            queue: "bugs".to_string(),
            item_id: "i1".to_string(),
            worker: "w".to_string(),
            project: String::new(),
        }
        .log_summary(),
        "queue:taken queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueCompleted {
            queue: "bugs".to_string(),
            item_id: "i1".to_string(),
            project: String::new(),
        }
        .log_summary(),
        "queue:completed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueFailed {
            queue: "bugs".to_string(),
            item_id: "i1".to_string(),
            error: "e".to_string(),
            project: String::new(),
        }
        .log_summary(),
        "queue:failed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueDropped {
            queue: "bugs".to_string(),
            item_id: "i1".to_string(),
            project: String::new(),
        }
        .log_summary(),
        "queue:dropped queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueRetry {
            queue: "bugs".to_string(),
            item_id: "i1".to_string(),
            project: String::new(),
        }
        .log_summary(),
        "queue:retry queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueDead {
            queue: "bugs".to_string(),
            item_id: "i1".to_string(),
            project: String::new(),
        }
        .log_summary(),
        "queue:dead queue=bugs item=i1"
    );
}

#[test]
fn log_summary_decision_created_job_owner() {
    let event = Event::DecisionCreated {
        id: DecisionId::from_string("d1"),
        agent_id: AgentId::from_string("a1"),
        owner: JobId::from_string("j1").into(),
        source: DecisionSource::Gate,
        context: "ctx".to_string(),
        options: vec![],
        questions: None,
        created_at_ms: 0,
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "decision:created id=d1 job=j1 source=Gate");
}

#[test]
fn log_summary_decision_created_crew_owner() {
    let event = Event::DecisionCreated {
        id: DecisionId::from_string("d1"),
        agent_id: AgentId::from_string("a1"),
        owner: CrewId::from_string("ar1").into(),
        source: DecisionSource::Question,
        context: "ctx".to_string(),
        options: vec![],
        questions: None,
        created_at_ms: 0,
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "decision:created id=d1 crew=ar1 source=Question");
}

#[test]
fn log_summary_decision_resolved_with_chosen() {
    let event = Event::DecisionResolved {
        id: DecisionId::from_string("d1"),
        choices: vec![2],
        message: None,
        resolved_at_ms: 0,
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "decision:resolved id=d1 chosen=2");
}

#[test]
fn log_summary_decision_resolved_no_chosen() {
    let event = Event::DecisionResolved {
        id: DecisionId::from_string("d1"),
        choices: vec![],
        message: Some("custom".to_string()),
        resolved_at_ms: 0,
        project: String::new(),
    };
    assert_eq!(event.log_summary(), "decision:resolved id=d1");
}

#[test]
fn log_summary_crew_created_no_namespace() {
    let event = Event::CrewCreated {
        id: CrewId::from_string("ar1"),
        agent: "builder".to_string(),
        command: "build".to_string(),
        project: String::new(),
        cwd: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        vars: HashMap::new(),
        created_at_ms: 0,
    };
    assert_eq!(event.log_summary(), "crew:created id=ar1 agent=builder");
}

#[test]
fn log_summary_crew_created_with_namespace() {
    let event = Event::CrewCreated {
        id: CrewId::from_string("ar1"),
        agent: "builder".to_string(),
        command: "build".to_string(),
        project: "prod".to_string(),
        cwd: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        vars: HashMap::new(),
        created_at_ms: 0,
    };
    assert_eq!(event.log_summary(), "crew:created id=ar1 ns=prod agent=builder");
}

#[test]
fn log_summary_crew_started() {
    let event =
        Event::CrewStarted { id: CrewId::from_string("ar1"), agent_id: AgentId::from_string("a1") };
    assert_eq!(event.log_summary(), "crew:started id=ar1 agent_id=a1");
}

#[test]
fn log_summary_crew_status_changed_with_reason() {
    let event = Event::CrewUpdated {
        id: CrewId::from_string("ar1"),
        status: CrewStatus::Failed,
        reason: Some("timeout".to_string()),
    };
    assert_eq!(event.log_summary(), "crew:updated id=ar1 status=failed reason=timeout");
}

#[test]
fn log_summary_crew_status_changed_no_reason() {
    let event = Event::CrewUpdated {
        id: CrewId::from_string("ar1"),
        status: CrewStatus::Running,
        reason: None,
    };
    assert_eq!(event.log_summary(), "crew:updated id=ar1 status=running");
}

#[test]
fn log_summary_crew_deleted() {
    let event = Event::CrewDeleted { id: CrewId::from_string("ar1") };
    assert_eq!(event.log_summary(), "crew:deleted id=ar1");
}

#[test]
fn log_summary_crew_resume_with_kill() {
    let event = Event::CrewResume {
        id: CrewId::from_string("ar1"),
        message: Some("retry".to_string()),
        kill: true,
    };
    assert_eq!(event.log_summary(), "crew:resume id=ar1 kill=true");
}

#[test]
fn log_summary_crew_resume_with_message() {
    let event = Event::CrewResume {
        id: CrewId::from_string("ar1"),
        message: Some("nudge".to_string()),
        kill: false,
    };
    assert_eq!(event.log_summary(), "crew:resume id=ar1 msg=true");
}

#[test]
fn log_summary_crew_resume_bare() {
    let event = Event::CrewResume { id: CrewId::from_string("ar1"), message: None, kill: false };
    assert_eq!(event.log_summary(), "crew:resume id=ar1");
}
