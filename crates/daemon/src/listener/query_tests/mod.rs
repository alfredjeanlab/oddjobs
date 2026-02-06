// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

mod entity_tests;
mod job_tests;
mod project_tests;
mod status_tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

use oj_core::{
    Decision, DecisionId, DecisionSource, Job, JobId, OwnerId, StepOutcome, StepRecord, StepStatus,
};
use oj_storage::{CronRecord, MaterializedState, QueueItem, QueueItemStatus, WorkerRecord};

use oj_engine::breadcrumb::{Breadcrumb, BreadcrumbAgent};

use crate::listener::ListenCtx;
use crate::protocol::{Query, Response};

use super::handle_query as real_handle_query;

/// Wrapper that constructs a ListenCtx from individual params for test convenience.
fn handle_query(
    query: Query,
    state: &Arc<Mutex<MaterializedState>>,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    logs_path: &std::path::Path,
    start_time: Instant,
) -> Response {
    let ctx = ListenCtx {
        event_bus: {
            let wal = oj_storage::Wal::open(&logs_path.join("__query_test.wal"), 0).unwrap();
            let (bus, _reader) = crate::event_bus::EventBus::new(wal);
            bus
        },
        state: Arc::clone(state),
        orphans: Arc::clone(orphans),
        metrics_health: Arc::new(Mutex::new(Default::default())),
        logs_path: logs_path.to_path_buf(),
        start_time,
        shutdown: Arc::new(tokio::sync::Notify::new()),
    };
    real_handle_query(&ctx, query)
}

fn empty_state() -> Arc<Mutex<MaterializedState>> {
    Arc::new(Mutex::new(MaterializedState::default()))
}

fn empty_orphans() -> Arc<Mutex<Vec<Breadcrumb>>> {
    Arc::new(Mutex::new(Vec::new()))
}

fn make_job(
    id: &str,
    name: &str,
    namespace: &str,
    step: &str,
    step_status: StepStatus,
    outcome: StepOutcome,
    agent_id: Option<&str>,
    started_at_ms: u64,
) -> Job {
    Job::builder()
        .id(id)
        .name(name)
        .kind("command")
        .namespace(namespace)
        .step(step)
        .step_status(step_status)
        .runbook_hash("")
        .cwd("")
        .step_history(vec![StepRecord {
            name: step.to_string(),
            started_at_ms,
            finished_at_ms: None,
            outcome,
            agent_id: agent_id.map(|s| s.to_string()),
            agent_name: None,
        }])
        .build()
}

fn make_worker(name: &str, namespace: &str, queue: &str, active: usize) -> WorkerRecord {
    WorkerRecord {
        name: name.to_string(),
        namespace: namespace.to_string(),
        project_root: std::path::PathBuf::from("/tmp"),
        runbook_hash: String::new(),
        status: "running".to_string(),
        active_job_ids: (0..active).map(|i| format!("p{}", i)).collect(),
        queue_name: queue.to_string(),
        concurrency: 3,
    }
}

fn make_queue_item(id: &str, status: QueueItemStatus) -> QueueItem {
    QueueItem {
        id: id.to_string(),
        queue_name: "merge".to_string(),
        data: HashMap::new(),
        status,
        worker_name: None,
        pushed_at_epoch_ms: 0,
        failure_count: 0,
    }
}

fn make_breadcrumb(job_id: &str, name: &str, project: &str, step: &str) -> Breadcrumb {
    Breadcrumb {
        job_id: job_id.to_string(),
        project: project.to_string(),
        kind: "command".to_string(),
        name: name.to_string(),
        vars: HashMap::new(),
        current_step: step.to_string(),
        step_status: "running".to_string(),
        agents: vec![BreadcrumbAgent {
            agent_id: "orphan-agent-1".to_string(),
            session_name: Some("tmux-orphan-1".to_string()),
            log_path: std::path::PathBuf::from("/tmp/agent.log"),
        }],
        workspace_id: None,
        workspace_root: Some(std::path::PathBuf::from("/tmp/ws")),
        updated_at: "2026-01-15T10:30:00Z".to_string(),
        runbook_hash: "hash123".to_string(),
        cwd: Some(std::path::PathBuf::from("/tmp/project")),
    }
}

fn make_cron(name: &str, namespace: &str, project_root: &str) -> CronRecord {
    CronRecord {
        name: name.to_string(),
        namespace: namespace.to_string(),
        project_root: std::path::PathBuf::from(project_root),
        runbook_hash: String::new(),
        status: "running".to_string(),
        interval: "5m".to_string(),
        run_target: "job:check".to_string(),
        started_at_ms: 0,
        last_fired_at_ms: None,
    }
}

fn make_decision(id: &str, job_id: &str, created_at_ms: u64) -> Decision {
    Decision {
        id: DecisionId::new(id),
        job_id: job_id.to_string(),
        agent_id: None,
        owner: OwnerId::Job(JobId::new(job_id)),
        source: DecisionSource::Idle,
        context: "test context".to_string(),
        options: vec![],
        chosen: None,
        message: None,
        created_at_ms,
        resolved_at_ms: None,
        superseded_by: None,
        namespace: "oddjobs".to_string(),
    }
}
