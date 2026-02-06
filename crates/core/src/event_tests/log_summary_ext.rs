// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for `Event::log_summary()` — timer, workspace, cron, worker, queue,
//! decision, and agent_run events.

use super::*;

#[test]
fn log_summary_timer_start() {
    let event = Event::TimerStart {
        id: TimerId::new("t1"),
    };
    assert_eq!(event.log_summary(), "timer:start id=t1");
}

#[test]
fn log_summary_workspace_events() {
    assert_eq!(
        Event::WorkspaceCreated {
            id: WorkspaceId::new("ws1"),
            path: PathBuf::from("/tmp/ws"),
            branch: Some("main".to_string()),
            owner: None,
            workspace_type: None,
        }
        .log_summary(),
        "workspace:created id=ws1"
    );
    assert_eq!(
        Event::WorkspaceReady {
            id: WorkspaceId::new("ws1"),
        }
        .log_summary(),
        "workspace:ready id=ws1"
    );
    assert_eq!(
        Event::WorkspaceFailed {
            id: WorkspaceId::new("ws1"),
            reason: "disk full".to_string(),
        }
        .log_summary(),
        "workspace:failed id=ws1"
    );
    assert_eq!(
        Event::WorkspaceDeleted {
            id: WorkspaceId::new("ws1"),
        }
        .log_summary(),
        "workspace:deleted id=ws1"
    );
    assert_eq!(
        Event::WorkspaceDrop {
            id: WorkspaceId::new("ws1"),
        }
        .log_summary(),
        "workspace:drop id=ws1"
    );
}

#[test]
fn log_summary_cron_started_stopped() {
    assert_eq!(
        Event::CronStarted {
            cron_name: "nightly".to_string(),
            project_root: PathBuf::from("/proj"),
            runbook_hash: "abc".to_string(),
            interval: "1h".to_string(),
            run_target: "job:build".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "cron:started cron=nightly"
    );
    assert_eq!(
        Event::CronStopped {
            cron_name: "nightly".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "cron:stopped cron=nightly"
    );
}

#[test]
fn log_summary_cron_once_job_target() {
    let event = Event::CronOnce {
        cron_name: "nightly".to_string(),
        job_id: JobId::new("j1"),
        job_name: "build".to_string(),
        job_kind: "build".to_string(),
        agent_run_id: None,
        agent_name: None,
        project_root: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        run_target: "job:build".to_string(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:once cron=nightly job=j1");
}

#[test]
fn log_summary_cron_once_agent_target() {
    let event = Event::CronOnce {
        cron_name: "nightly".to_string(),
        job_id: JobId::default(),
        job_name: String::new(),
        job_kind: String::new(),
        agent_run_id: Some("ar1".to_string()),
        agent_name: Some("builder".to_string()),
        project_root: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        run_target: "agent:builder".to_string(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:once cron=nightly agent=builder");
}

#[test]
fn log_summary_cron_fired_job() {
    let event = Event::CronFired {
        cron_name: "nightly".to_string(),
        job_id: JobId::new("j1"),
        agent_run_id: None,
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:fired cron=nightly job=j1");
}

#[test]
fn log_summary_cron_fired_agent_run() {
    let event = Event::CronFired {
        cron_name: "nightly".to_string(),
        job_id: JobId::default(),
        agent_run_id: Some("ar1".to_string()),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:fired cron=nightly agent_run=ar1");
}

#[test]
fn log_summary_cron_deleted_no_namespace() {
    let event = Event::CronDeleted {
        cron_name: "nightly".to_string(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:deleted cron=nightly");
}

#[test]
fn log_summary_cron_deleted_with_namespace() {
    let event = Event::CronDeleted {
        cron_name: "nightly".to_string(),
        namespace: "prod".to_string(),
    };
    assert_eq!(event.log_summary(), "cron:deleted cron=nightly ns=prod");
}

#[test]
fn log_summary_worker_events() {
    assert_eq!(
        Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: PathBuf::from("/proj"),
            runbook_hash: "abc".to_string(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        }
        .log_summary(),
        "worker:started worker=fixer"
    );
    assert_eq!(
        Event::WorkerWake {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "worker:wake worker=fixer"
    );
    assert_eq!(
        Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "worker:stopped worker=fixer"
    );
}

#[test]
fn log_summary_worker_poll_complete() {
    let event = Event::WorkerPollComplete {
        worker_name: "fixer".to_string(),
        items: vec![
            serde_json::json!({"id": "1"}),
            serde_json::json!({"id": "2"}),
        ],
    };
    assert_eq!(
        event.log_summary(),
        "worker:poll_complete worker=fixer items=2"
    );
}

#[test]
fn log_summary_worker_take_complete() {
    let event = Event::WorkerTakeComplete {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        item: serde_json::json!({}),
        exit_code: 0,
        stderr: None,
    };
    assert_eq!(
        event.log_summary(),
        "worker:take_complete worker=fixer item=item-1 exit=0"
    );
}

#[test]
fn log_summary_worker_item_dispatched() {
    let event = Event::WorkerItemDispatched {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        job_id: JobId::new("j1"),
        namespace: String::new(),
    };
    assert_eq!(
        event.log_summary(),
        "worker:item_dispatched worker=fixer item=item-1 job=j1"
    );
}

#[test]
fn log_summary_worker_resized_no_namespace() {
    let event = Event::WorkerResized {
        worker_name: "fixer".to_string(),
        concurrency: 4,
        namespace: String::new(),
    };
    assert_eq!(
        event.log_summary(),
        "worker:resized worker=fixer concurrency=4"
    );
}

#[test]
fn log_summary_worker_resized_with_namespace() {
    let event = Event::WorkerResized {
        worker_name: "fixer".to_string(),
        concurrency: 4,
        namespace: "prod".to_string(),
    };
    assert_eq!(
        event.log_summary(),
        "worker:resized worker=fixer ns=prod concurrency=4"
    );
}

#[test]
fn log_summary_worker_deleted_no_namespace() {
    let event = Event::WorkerDeleted {
        worker_name: "fixer".to_string(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "worker:deleted worker=fixer");
}

#[test]
fn log_summary_worker_deleted_with_namespace() {
    let event = Event::WorkerDeleted {
        worker_name: "fixer".to_string(),
        namespace: "prod".to_string(),
    };
    assert_eq!(event.log_summary(), "worker:deleted worker=fixer ns=prod");
}

#[test]
fn log_summary_queue_events() {
    assert_eq!(
        Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            data: HashMap::new(),
            pushed_at_epoch_ms: 0,
            namespace: String::new(),
        }
        .log_summary(),
        "queue:pushed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            worker_name: "w".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:taken queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueCompleted {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:completed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueFailed {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            error: "e".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:failed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueDropped {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:dropped queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueItemRetry {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:item_retry queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueItemDead {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:item_dead queue=bugs item=i1"
    );
}

#[test]
fn log_summary_decision_created_job_owner() {
    let event = Event::DecisionCreated {
        id: "d1".to_string(),
        job_id: JobId::new("j1"),
        agent_id: None,
        owner: OwnerId::Job(JobId::new("j1")),
        source: DecisionSource::Gate,
        context: "ctx".to_string(),
        options: vec![],
        created_at_ms: 0,
        namespace: String::new(),
    };
    assert_eq!(
        event.log_summary(),
        "decision:created id=d1 job=j1 source=Gate"
    );
}

#[test]
fn log_summary_decision_created_agent_run_owner() {
    use crate::agent_run::AgentRunId;
    use crate::owner::OwnerId;
    let event = Event::DecisionCreated {
        id: "d1".to_string(),
        job_id: JobId::default(),
        agent_id: None,
        owner: OwnerId::AgentRun(AgentRunId::new("ar1")),
        source: DecisionSource::Question,
        context: "ctx".to_string(),
        options: vec![],
        created_at_ms: 0,
        namespace: String::new(),
    };
    assert_eq!(
        event.log_summary(),
        "decision:created id=d1 agent_run=ar1 source=Question"
    );
}

#[test]
fn log_summary_decision_resolved_with_chosen() {
    let event = Event::DecisionResolved {
        id: "d1".to_string(),
        chosen: Some(2),
        message: None,
        resolved_at_ms: 0,
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "decision:resolved id=d1 chosen=2");
}

#[test]
fn log_summary_decision_resolved_no_chosen() {
    let event = Event::DecisionResolved {
        id: "d1".to_string(),
        chosen: None,
        message: Some("custom".to_string()),
        resolved_at_ms: 0,
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "decision:resolved id=d1");
}

#[test]
fn log_summary_agent_run_created_no_namespace() {
    use crate::agent_run::AgentRunId;
    let event = Event::AgentRunCreated {
        id: AgentRunId::new("ar1"),
        agent_name: "builder".to_string(),
        command_name: "build".to_string(),
        namespace: String::new(),
        cwd: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        vars: HashMap::new(),
        created_at_epoch_ms: 0,
    };
    assert_eq!(
        event.log_summary(),
        "agent_run:created id=ar1 agent=builder"
    );
}

#[test]
fn log_summary_agent_run_created_with_namespace() {
    use crate::agent_run::AgentRunId;
    let event = Event::AgentRunCreated {
        id: AgentRunId::new("ar1"),
        agent_name: "builder".to_string(),
        command_name: "build".to_string(),
        namespace: "prod".to_string(),
        cwd: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        vars: HashMap::new(),
        created_at_epoch_ms: 0,
    };
    assert_eq!(
        event.log_summary(),
        "agent_run:created id=ar1 ns=prod agent=builder"
    );
}

#[test]
fn log_summary_agent_run_started() {
    use crate::agent_run::AgentRunId;
    let event = Event::AgentRunStarted {
        id: AgentRunId::new("ar1"),
        agent_id: AgentId::new("a1"),
    };
    assert_eq!(event.log_summary(), "agent_run:started id=ar1 agent_id=a1");
}

#[test]
fn log_summary_agent_run_status_changed_with_reason() {
    use crate::agent_run::{AgentRunId, AgentRunStatus};
    let event = Event::AgentRunStatusChanged {
        id: AgentRunId::new("ar1"),
        status: AgentRunStatus::Failed,
        reason: Some("timeout".to_string()),
    };
    assert_eq!(
        event.log_summary(),
        "agent_run:status_changed id=ar1 status=failed reason=timeout"
    );
}

#[test]
fn log_summary_agent_run_status_changed_no_reason() {
    use crate::agent_run::{AgentRunId, AgentRunStatus};
    let event = Event::AgentRunStatusChanged {
        id: AgentRunId::new("ar1"),
        status: AgentRunStatus::Running,
        reason: None,
    };
    assert_eq!(
        event.log_summary(),
        "agent_run:status_changed id=ar1 status=running"
    );
}

#[test]
fn log_summary_agent_run_deleted() {
    use crate::agent_run::AgentRunId;
    let event = Event::AgentRunDeleted {
        id: AgentRunId::new("ar1"),
    };
    assert_eq!(event.log_summary(), "agent_run:deleted id=ar1");
}
