// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared test fixtures for listener tests (mutations + queries).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{
    AgentId, CrewStatus, Decision, DecisionId, DecisionSource, Event, Job, JobId, StepOutcome,
    StepRecord, StepStatus, WorkspaceStatus,
};
use oj_engine::breadcrumb::{Breadcrumb, BreadcrumbAgent};
use oj_storage::{
    CronRecord, MaterializedState, QueueItem, QueueItemStatus, WorkerRecord, Workspace,
    WorkspaceType,
};

pub fn make_job(id: &str, step: &str) -> Job {
    Job::builder()
        .id(id)
        .kind("test")
        .project("proj")
        .step(step)
        .runbook_hash("abc123")
        .cwd("/tmp/project")
        .step_history(vec![StepRecord {
            name: step.to_string(),
            started_at_ms: 1000,
            finished_at_ms: None,
            outcome: StepOutcome::Running,
            agent_id: None,
            agent_name: None,
        }])
        .build()
}

pub fn make_job_ns(id: &str, step: &str, project: &str) -> Job {
    let mut p = make_job(id, step);
    p.project = project.to_string();
    p
}

pub fn make_job_with_agent(id: &str, step: &str, agent_id: &str) -> Job {
    Job::builder()
        .id(id)
        .kind("test")
        .project("proj")
        .step(step)
        .runbook_hash("abc123")
        .cwd("/tmp/project")
        .step_history(vec![StepRecord {
            name: "work".to_string(),
            started_at_ms: 1000,
            finished_at_ms: Some(2000),
            outcome: StepOutcome::Completed,
            agent_id: Some(agent_id.to_string()),
            agent_name: Some("test-agent".to_string()),
        }])
        .build()
}

pub fn make_job_agent_in_history(
    id: &str,
    current_step: &str,
    agent_step: &str,
    agent_id: &str,
) -> Job {
    Job::builder()
        .id(id)
        .kind("test")
        .project("proj")
        .step(current_step)
        .runbook_hash("abc123")
        .cwd("/tmp/project")
        .step_history(vec![
            StepRecord {
                name: agent_step.to_string(),
                started_at_ms: 1000,
                finished_at_ms: Some(2000),
                outcome: StepOutcome::Completed,
                agent_id: Some(agent_id.to_string()),
                agent_name: Some("test-agent".to_string()),
            },
            StepRecord {
                name: current_step.to_string(),
                started_at_ms: 2000,
                finished_at_ms: None,
                outcome: StepOutcome::Running,
                agent_id: None,
                agent_name: None,
            },
        ])
        .build()
}

pub fn make_breadcrumb(job_id: &str) -> Breadcrumb {
    Breadcrumb {
        job_id: job_id.to_string(),
        project: "proj".to_string(),
        kind: "test".to_string(),
        name: "test-job".to_string(),
        vars: HashMap::new(),
        current_step: "work".to_string(),
        step_status: "running".to_string(),
        agents: vec![],
        workspace_id: None,
        workspace_root: None,
        updated_at: "2026-01-15T10:30:00Z".to_string(),
        runbook_hash: "hash456".to_string(),
        cwd: Some(std::path::PathBuf::from("/tmp/project")),
    }
}

pub fn make_crew(id: &str, status: CrewStatus) -> oj_core::Crew {
    oj_core::Crew::builder()
        .id(id)
        .agent_name("test-agent")
        .command_name("test-cmd")
        .project("proj")
        .cwd("/tmp/project")
        .runbook_hash("hash123")
        .status(status)
        .agent_id(format!("{}-agent-uuid", id))
        .build()
}

pub fn make_agent_runbook_json(job_kind: &str, step_name: &str) -> serde_json::Value {
    serde_json::json!({
        "jobs": {
            job_kind: {
                "kind": job_kind,
                "steps": [{ "name": step_name, "run": { "agent": "test-agent" } }]
            }
        }
    })
}

pub fn make_shell_runbook_json(job_kind: &str, step_name: &str) -> serde_json::Value {
    serde_json::json!({
        "jobs": {
            job_kind: {
                "kind": job_kind,
                "steps": [{ "name": step_name, "run": "echo hello" }]
            }
        }
    })
}

pub fn load_runbook_into_state(state: &Arc<Mutex<MaterializedState>>, hash: &str) {
    let event =
        Event::RunbookLoaded { hash: hash.to_string(), version: 1, runbook: serde_json::json!({}) };
    state.lock().apply_event(&event);
}

pub fn load_runbook_json_into_state(
    state: &Arc<Mutex<MaterializedState>>,
    hash: &str,
    runbook_json: serde_json::Value,
) {
    let event = Event::RunbookLoaded { hash: hash.to_string(), version: 1, runbook: runbook_json };
    state.lock().apply_event(&event);
}

pub fn make_workspace(id: &str, path: std::path::PathBuf, owner: Option<&str>) -> Workspace {
    let owner_id = owner.unwrap_or(id);
    Workspace {
        id: id.to_string(),
        path,
        branch: None,
        owner: JobId::new(owner_id).into(),
        status: WorkspaceStatus::Ready,
        workspace_type: WorkspaceType::default(),
        created_at_ms: 0,
    }
}

pub fn make_job_full(
    id: &str,
    name: &str,
    project: &str,
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
        .project(project)
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

pub fn make_breadcrumb_full(job_id: &str, name: &str, project: &str, step: &str) -> Breadcrumb {
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
            session_name: Some("oj-orphan-1".to_string()),
            log_path: std::path::PathBuf::from("/tmp/agent.log"),
        }],
        workspace_id: None,
        workspace_root: Some(std::path::PathBuf::from("/tmp/ws")),
        updated_at: "2026-01-15T10:30:00Z".to_string(),
        runbook_hash: "hash123".to_string(),
        cwd: Some(std::path::PathBuf::from("/tmp/project")),
    }
}

pub fn make_worker(name: &str, project: &str, queue: &str, active: usize) -> WorkerRecord {
    WorkerRecord {
        name: name.to_string(),
        project: project.to_string(),
        project_path: std::path::PathBuf::from("/tmp"),
        runbook_hash: String::new(),
        status: "running".to_string(),
        active: (0..active).map(|i| format!("p{}", i)).collect(),
        queue: queue.to_string(),
        concurrency: 3,
        owners: HashMap::new(),
    }
}

pub fn make_queue_item(id: &str, status: QueueItemStatus) -> QueueItem {
    QueueItem {
        id: id.to_string(),
        queue: "merge".to_string(),
        data: HashMap::new(),
        status,
        worker: None,
        pushed_at_ms: 0,
        failures: 0,
    }
}

pub fn make_cron(name: &str, project: &str, project_path: &str) -> CronRecord {
    CronRecord {
        name: name.to_string(),
        project: project.to_string(),
        project_path: std::path::PathBuf::from(project_path),
        runbook_hash: String::new(),
        status: "running".to_string(),
        interval: "5m".to_string(),
        target: oj_core::RunTarget::job("check"),
        started_at_ms: 0,
        last_fired_at_ms: None,
    }
}

pub fn make_decision(id: &str, job_id: &str, created_at_ms: u64) -> Decision {
    Decision {
        id: DecisionId::new(id),
        agent_id: AgentId::new("test-agent"),
        owner: JobId::new(job_id).into(),
        source: DecisionSource::Idle,
        context: "test context".to_string(),
        options: vec![],
        questions: None,
        choices: vec![],
        message: None,
        created_at_ms,
        resolved_at_ms: None,
        superseded_by: None,
        project: "oddjobs".to_string(),
    }
}
