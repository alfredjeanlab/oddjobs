// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Materialized state from WAL replay

mod agents;
mod decisions;
mod helpers;
mod jobs;
mod queues;
mod types;
mod workers;
mod workspaces;

#[cfg(test)]
pub use types::WorkspaceType;
pub use types::{
    CronRecord, QueueItem, QueueItemStatus, QueuePollMeta, StoredRunbook, WorkerRecord, Workspace,
};

use oj_core::{AgentRecord, Crew, Decision, Event, Job};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Materialized state built from WAL operations
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MaterializedState {
    pub jobs: HashMap<String, Job>,
    pub workspaces: HashMap<String, Workspace>,
    #[serde(default)]
    pub runbooks: HashMap<String, StoredRunbook>,
    #[serde(default)]
    pub workers: HashMap<String, WorkerRecord>,
    #[serde(default)]
    pub queue_items: HashMap<String, Vec<QueueItem>>,
    #[serde(default)]
    pub crons: HashMap<String, CronRecord>,
    #[serde(default)]
    pub decisions: HashMap<String, Decision>,
    #[serde(default)]
    pub crew: HashMap<String, Crew>,
    /// Unified agent index: agent_id → AgentRecord.
    ///
    /// Populated from existing events (StepStarted, CrewStarted, agent
    /// state events) during WAL replay. Provides a single source of truth
    /// for all agent queries regardless of whether the agent is job-embedded
    /// or standalone.
    #[serde(default)]
    pub agents: HashMap<String, AgentRecord>,
    /// Runtime-only poll metadata: scoped_queue_key → last poll info.
    /// Not persisted — repopulates naturally as workers resume polling.
    #[serde(skip)]
    pub poll_meta: HashMap<String, QueuePollMeta>,
    /// Durable project → project path mapping.
    ///
    /// Populated from WorkerStarted, CronStarted, and CommandRun events.
    /// Never cleared by deletion events, so the mapping survives worker/cron pruning.
    #[serde(default)]
    pub project_paths: HashMap<String, PathBuf>,
}

impl MaterializedState {
    /// Get a job by ID or unique prefix (like git commit hashes)
    pub fn get_job(&self, id: &str) -> Option<&Job> {
        helpers::find_by_prefix(&self.jobs, id)
    }

    /// Get a decision by ID or unique prefix
    pub fn get_decision(&self, id: &str) -> Option<&Decision> {
        helpers::find_by_prefix(&self.decisions, id)
    }

    /// Look up the known project path for a project.
    ///
    /// Checks the durable project_paths map first (survives worker/cron pruning),
    /// then falls back to scanning active workers and crons.
    pub fn project_path_for_namespace(&self, project: &str) -> Option<std::path::PathBuf> {
        if let Some(root) = self.project_paths.get(project) {
            return Some(root.clone());
        }
        for w in self.workers.values() {
            if w.project == project {
                return Some(w.project_path.clone());
            }
        }
        for c in self.crons.values() {
            if c.project == project {
                return Some(c.project_path.clone());
            }
        }
        None
    }

    /// Apply an event to derive state changes.
    ///
    /// This is the event-sourcing approach where state is derived from events.
    /// Events are facts about what happened; state is derived from those facts.
    ///
    /// # Idempotency Requirement
    ///
    /// **All event handlers MUST be idempotent.** Applying the same event twice
    /// must produce the same state as applying it once. This is critical because
    /// events may be applied multiple times:
    ///
    /// 1. In `executor.execute_inner()` for immediate visibility
    /// 2. In `daemon.process_event()` after WAL replay
    ///
    /// Guidelines for idempotent handlers:
    /// - Use assignment (`=`) instead of mutation (`+=`, `-=`)
    /// - Guard inserts with existence checks (`if !map.contains_key(...)`)
    /// - Guard increments with status checks (only increment on state transition)
    /// - Use `finalize_current_step` which is internally guarded by `finished_at_ms`
    pub fn apply_event(&mut self, event: &Event) {
        match event {
            // Agent lifecycle
            Event::AgentWorking { .. }
            | Event::AgentWaiting { .. }
            | Event::AgentExited { .. }
            | Event::AgentFailed { .. }
            | Event::AgentGone { .. } => agents::apply(self, event),

            // Jobs, steps, and shell
            Event::JobCreated { .. }
            | Event::RunbookLoaded { .. }
            | Event::JobAdvanced { .. }
            | Event::StepStarted { .. }
            | Event::StepWaiting { .. }
            | Event::StepCompleted { .. }
            | Event::StepFailed { .. }
            | Event::JobFailing { .. }
            | Event::JobCancelling { .. }
            | Event::JobSuspending { .. }
            | Event::JobDeleted { .. }
            | Event::ShellExited { .. }
            | Event::JobUpdated { .. } => jobs::apply(self, event),

            // Workspaces
            Event::WorkspaceCreated { .. }
            | Event::WorkspaceReady { .. }
            | Event::WorkspaceFailed { .. }
            | Event::WorkspaceDeleted { .. } => workspaces::apply(self, event),

            // AgentSpawned: persist the runtime adapter type on the agent record
            Event::AgentSpawned { .. } => agents::apply(self, event),

            // Workers and crons
            Event::WorkerStarted { .. }
            | Event::WorkerDispatched { .. }
            | Event::WorkerStopped { .. }
            | Event::WorkerResized { .. }
            | Event::WorkerDeleted { .. }
            | Event::CronStarted { .. }
            | Event::CronStopped { .. }
            | Event::CronFired { .. }
            | Event::CronDeleted { .. } => workers::apply(self, event),

            // Queues
            Event::QueuePushed { .. }
            | Event::QueueTaken { .. }
            | Event::QueueCompleted { .. }
            | Event::QueueFailed { .. }
            | Event::QueueDropped { .. }
            | Event::QueueRetry { .. }
            | Event::QueueDead { .. } => queues::apply(self, event),

            // Decisions, crew, and commands
            Event::DecisionCreated { .. }
            | Event::DecisionResolved { .. }
            | Event::CrewCreated { .. }
            | Event::CrewStarted { .. }
            | Event::CrewUpdated { .. }
            | Event::CrewDeleted { .. }
            | Event::CommandRun { .. } => decisions::apply(self, event),

            // Events that don't affect persisted state
            // (These are action/signal events handled by the runtime)
            Event::Custom
            | Event::TimerStart { .. }
            | Event::AgentInput { .. }
            | Event::AgentRespond { .. }
            | Event::AgentSpawnFailed { .. }
            | Event::JobResume { .. }
            | Event::JobCancel { .. }
            | Event::JobSuspend { .. }
            | Event::CrewResume { .. }
            | Event::WorkspaceDrop { .. }
            | Event::WorkerWake { .. }
            | Event::WorkerPolled { .. }
            | Event::WorkerTook { .. }
            | Event::AgentIdle { .. }
            | Event::AgentPrompt { .. }
            | Event::AgentStopBlocked { .. }
            | Event::AgentStopAllowed { .. }
            | Event::CronOnce { .. }
            | Event::Shutdown => {}
        }
    }
}

#[cfg(test)]
#[path = "../state_tests/mod.rs"]
mod tests;
