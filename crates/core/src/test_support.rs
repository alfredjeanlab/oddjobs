// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared test helpers for use across crates.
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]`.

use crate::{AgentId, CrewId, Event, JobId};
use std::collections::HashMap;
use std::path::PathBuf;

// ── Proptest strategies ─────────────────────────────────────────────────

/// Proptest strategies for core state machine types.
pub mod strategies {
    use crate::job::StepStatus;
    use proptest::prelude::*;

    pub fn arb_step_status() -> impl Strategy<Value = StepStatus> {
        prop_oneof![
            Just(StepStatus::Pending),
            Just(StepStatus::Running),
            any::<Option<String>>().prop_map(StepStatus::Waiting),
            Just(StepStatus::Completed),
            Just(StepStatus::Failed),
            Just(StepStatus::Suspended),
        ]
    }
}

// ── Event factory functions ─────────────────────────────────────────────────

pub fn job_create_event(id: &str, kind: &str, name: &str, initial_step: &str) -> Event {
    Event::JobCreated {
        id: JobId::new(id),
        kind: kind.to_string(),
        name: name.to_string(),
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/test/project"),
        vars: HashMap::new(),
        initial_step: initial_step.to_string(),
        created_at_ms: 1_000_000,
        project: String::new(),
        cron: None,
    }
}

pub fn job_delete_event(id: &str) -> Event {
    Event::JobDeleted { id: JobId::new(id) }
}

pub fn job_transition_event(id: &str, step: &str) -> Event {
    Event::JobAdvanced { id: JobId::new(id), step: step.to_string() }
}

pub fn step_started_event(job_id: &str) -> Event {
    Event::StepStarted {
        job_id: JobId::new(job_id),
        step: "init".to_string(),
        agent_id: None,
        agent_name: None,
    }
}

pub fn step_failed_event(job_id: &str, step: &str, error: &str) -> Event {
    Event::StepFailed {
        job_id: JobId::new(job_id),
        step: step.to_string(),
        error: error.to_string(),
    }
}

pub fn agent_spawned_event(agent_id: &str, job_id: &str) -> Event {
    Event::AgentSpawned { id: AgentId::new(agent_id), owner: JobId::new(job_id).into() }
}

pub fn worker_start_event(name: &str, project: &str) -> Event {
    Event::WorkerStarted {
        worker: name.to_string(),
        project_path: PathBuf::from("/test/project"),
        runbook_hash: "abc123".to_string(),
        queue: "queue".to_string(),
        concurrency: 1,
        project: project.to_string(),
    }
}

pub fn queue_pushed_event(queue: &str, item_id: &str) -> Event {
    Event::QueuePushed {
        queue: queue.to_string(),
        item_id: item_id.to_string(),
        data: [("title".to_string(), "Fix bug".to_string()), ("id".to_string(), "123".to_string())]
            .into_iter()
            .collect(),
        pushed_at_ms: 1_000_000,
        project: String::new(),
    }
}

pub fn queue_taken_event(queue: &str, item_id: &str, worker: &str) -> Event {
    Event::QueueTaken {
        project: String::new(),
        worker: worker.to_string(),
        queue: queue.to_string(),
        item_id: item_id.to_string(),
    }
}

pub fn queue_failed_event(queue: &str, item_id: &str, error: &str) -> Event {
    Event::QueueFailed {
        queue: queue.to_string(),
        item_id: item_id.to_string(),
        error: error.to_string(),
        project: String::new(),
    }
}

pub fn crew_created_event(id: &str, agent_name: &str, command_name: &str) -> Event {
    Event::CrewCreated {
        id: CrewId::new(id),
        agent: agent_name.to_string(),
        command: command_name.to_string(),
        project: String::new(),
        cwd: PathBuf::from("/test/project"),
        runbook_hash: "testhash".to_string(),
        vars: HashMap::new(),
        created_at_ms: 1_000_000,
    }
}
