// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{AgentRunStatus, Event, Job, StepOutcome, StepRecord, WorkspaceStatus};
use oj_engine::breadcrumb::Breadcrumb;
use oj_storage::{MaterializedState, Workspace, WorkspaceType};

// Re-export for sibling test modules (private items visible to descendants)
pub fn test_ctx(dir: &std::path::Path) -> crate::listener::ListenCtx {
    super::super::test_ctx(dir)
}

pub fn make_job(id: &str, step: &str) -> Job {
    Job::builder()
        .id(id)
        .kind("test")
        .namespace("proj")
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

pub fn load_runbook_into_state(state: &Arc<Mutex<MaterializedState>>, hash: &str) {
    let event = Event::RunbookLoaded {
        hash: hash.to_string(),
        version: 1,
        runbook: serde_json::json!({}),
    };
    state.lock().apply_event(&event);
}

pub fn make_job_with_agent(id: &str, step: &str, agent_id: &str) -> Job {
    Job::builder()
        .id(id)
        .kind("test")
        .namespace("proj")
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

pub fn make_agent_run(id: &str, status: AgentRunStatus) -> oj_core::AgentRun {
    oj_core::AgentRun::builder()
        .id(id)
        .agent_name("test-agent")
        .command_name("test-cmd")
        .namespace("proj")
        .cwd("/tmp/project")
        .runbook_hash("hash123")
        .status(status)
        .agent_id(format!("{}-agent-uuid", id))
        .session_id(format!("oj-{}", id))
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

pub fn load_runbook_json_into_state(
    state: &Arc<Mutex<MaterializedState>>,
    hash: &str,
    runbook_json: serde_json::Value,
) {
    let event = Event::RunbookLoaded {
        hash: hash.to_string(),
        version: 1,
        runbook: runbook_json,
    };
    state.lock().apply_event(&event);
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
        .namespace("proj")
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

pub fn make_job_ns(id: &str, step: &str, namespace: &str) -> Job {
    let mut p = make_job(id, step);
    p.namespace = namespace.to_string();
    p
}

pub fn make_workspace(id: &str, path: std::path::PathBuf, owner: Option<&str>) -> Workspace {
    Workspace {
        id: id.to_string(),
        path,
        branch: None,
        owner: owner.map(|o| oj_core::OwnerId::Job(oj_core::JobId::new(o))),
        status: WorkspaceStatus::Ready,
        workspace_type: WorkspaceType::default(),
        created_at_ms: 0,
    }
}
