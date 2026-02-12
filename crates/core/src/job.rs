// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job identifier and state machine.

use crate::actions::ActionTracker;
use crate::clock::Clock;
use crate::workspace::WorkspaceId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

crate::define_id! {
    /// Unique identifier for a job instance.
    ///
    /// Each job run gets a unique ID that can be used to track its state,
    /// query its status, and reference it in logs and events.
    #[derive(Default)]
    pub struct JobId;
}

/// Status of the current step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    /// Waiting to start
    Pending,
    /// Agent is running
    Running,
    /// Waiting for external input (optional decision_id)
    Waiting(Option<String>),
    /// Step completed
    Completed,
    /// Step failed
    Failed,
    /// Job suspended (terminal but resumable)
    Suspended,
}

impl StepStatus {
    /// Check if this step is in a waiting state.
    pub fn is_waiting(&self) -> bool {
        matches!(self, StepStatus::Waiting(_))
    }

    /// Check if this step is suspended.
    pub fn is_suspended(&self) -> bool {
        matches!(self, StepStatus::Suspended)
    }
}

crate::simple_display! {
    StepStatus {
        Pending => "pending",
        Running => "running",
        Waiting(..) => "waiting",
        Completed => "completed",
        Failed => "failed",
        Suspended => "suspended",
    }
}

/// Outcome of a completed or in-progress step (for step history)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepOutcome {
    Running,
    Completed,
    Failed(String),
    Waiting(String),
}

/// Tag-only variant of [`StepStatus`] for protocol DTOs (strips associated data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatusKind {
    Pending,
    Running,
    Waiting,
    Completed,
    Failed,
    Suspended,
    /// Orphaned job detected from breadcrumb (not a core step status).
    Orphaned,
}

impl From<&StepStatus> for StepStatusKind {
    fn from(s: &StepStatus) -> Self {
        match s {
            StepStatus::Pending => StepStatusKind::Pending,
            StepStatus::Running => StepStatusKind::Running,
            StepStatus::Waiting(_) => StepStatusKind::Waiting,
            StepStatus::Completed => StepStatusKind::Completed,
            StepStatus::Failed => StepStatusKind::Failed,
            StepStatus::Suspended => StepStatusKind::Suspended,
        }
    }
}

crate::simple_display! {
    StepStatusKind {
        Pending => "pending",
        Running => "running",
        Waiting => "waiting",
        Completed => "completed",
        Failed => "failed",
        Suspended => "suspended",
        Orphaned => "orphaned",
    }
}

/// Tag-only variant of [`StepOutcome`] for protocol DTOs (strips associated data).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcomeKind {
    Running,
    Completed,
    Failed,
    Waiting,
}

impl From<&StepOutcome> for StepOutcomeKind {
    fn from(o: &StepOutcome) -> Self {
        match o {
            StepOutcome::Running => StepOutcomeKind::Running,
            StepOutcome::Completed => StepOutcomeKind::Completed,
            StepOutcome::Failed(_) => StepOutcomeKind::Failed,
            StepOutcome::Waiting(_) => StepOutcomeKind::Waiting,
        }
    }
}

crate::simple_display! {
    StepOutcomeKind {
        Running => "running",
        Completed => "completed",
        Failed => "failed",
        Waiting => "waiting",
    }
}

/// Record of a step execution (for step history)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    pub name: String,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub outcome: StepOutcome,
    /// Agent ID that ran this step (if any)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Agent name from the runbook definition (if any)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
}

/// Configuration for creating a new job
#[derive(Debug, Clone)]
pub struct JobConfig {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub vars: HashMap<String, String>,
    pub runbook_hash: String,
    pub cwd: PathBuf,
    pub initial_step: String,
    pub project: String,
    pub cron_name: Option<String>,
}

impl JobConfig {
    pub fn builder(
        id: impl Into<String>,
        kind: impl Into<String>,
        initial_step: impl Into<String>,
    ) -> JobConfigBuilder {
        let id = id.into();
        JobConfigBuilder {
            id: id.clone(),
            name: id,
            kind: kind.into(),
            vars: HashMap::new(),
            runbook_hash: String::new(),
            cwd: PathBuf::new(),
            initial_step: initial_step.into(),
            project: String::new(),
            cron_name: None,
        }
    }
}

pub struct JobConfigBuilder {
    id: String,
    name: String,
    kind: String,
    vars: HashMap<String, String>,
    runbook_hash: String,
    cwd: PathBuf,
    initial_step: String,
    project: String,
    cron_name: Option<String>,
}

impl JobConfigBuilder {
    crate::setters! {
        into {
            name: String,
            runbook_hash: String,
            cwd: PathBuf,
            project: String,
        }
        set {
            vars: HashMap<String, String>,
        }
        option {
            cron_name: String,
        }
    }

    pub fn build(self) -> JobConfig {
        JobConfig {
            id: self.id,
            name: self.name,
            kind: self.kind,
            vars: self.vars,
            runbook_hash: self.runbook_hash,
            cwd: self.cwd,
            initial_step: self.initial_step,
            project: self.project,
            cron_name: self.cron_name,
        }
    }
}

/// Maximum number of times any single step can be entered before the job
/// is failed with a circuit-breaker error. Prevents runaway retry cycles
/// (e.g., merge → resolve → push → reinit → merge looping indefinitely).
pub const MAX_STEP_VISITS: u32 = 5;

/// A job instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// Project project this job belongs to
    pub project: String,
    /// Current step name (from runbook definition)
    pub step: String,
    pub step_status: StepStatus,
    #[serde(skip, default = "Instant::now")]
    pub step_started_at: Instant,
    #[serde(default)]
    pub step_history: Vec<StepRecord>,
    pub vars: HashMap<String, String>,
    /// Content hash of the stored runbook (for cache lookup)
    pub runbook_hash: String,
    /// Current working directory where commands execute
    pub cwd: PathBuf,
    /// Reference to the workspace this job is using (for managed git worktrees)
    pub workspace_id: Option<WorkspaceId>,
    /// Path to the workspace (derived from workspace_id lookup)
    pub workspace_path: Option<PathBuf>,
    #[serde(skip, default = "Instant::now")]
    pub created_at: Instant,
    pub error: Option<String>,
    /// Action attempt tracking and agent signal state.
    #[serde(flatten)]
    pub actions: ActionTracker,
    /// True when running an on_cancel cleanup step. Prevents re-cancellation.
    #[serde(default)]
    pub cancelling: bool,
    /// True when running an on_fail cleanup step. Routes to "failed" on completion.
    #[serde(default)]
    pub failing: bool,
    /// True when suspension is in progress. Prevents re-suspension.
    #[serde(default)]
    pub suspending: bool,
    /// Cumulative retry count across all steps (incremented each time an action
    /// is re-attempted, i.e. when attempt count > 1).
    #[serde(default)]
    pub total_retries: u32,
    /// Tracks how many times each step has been entered.
    /// Used as a circuit breaker to prevent runaway retry cycles.
    #[serde(default)]
    pub step_visits: HashMap<String, u32>,
    /// Name of the cron that spawned this job, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_name: Option<String>,
    /// Epoch milliseconds when the last nudge was sent.
    /// Used to suppress auto-resume from our own nudge text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_nudge_at: Option<u64>,
}

impl Job {
    /// Create a new job with the given initial step
    pub fn new(config: JobConfig, clock: &impl Clock) -> Self {
        Self::new_with_epoch_ms(config, clock.epoch_ms())
    }

    /// Create a new job with explicit epoch_ms (for WAL replay)
    pub fn new_with_epoch_ms(config: JobConfig, epoch_ms: u64) -> Self {
        Self {
            id: config.id,
            name: config.name,
            kind: config.kind,
            project: config.project,
            step: config.initial_step.clone(),
            step_status: StepStatus::Pending,
            vars: config.vars,
            runbook_hash: config.runbook_hash,
            cwd: config.cwd,
            workspace_id: None,
            workspace_path: None,
            created_at: Instant::now(),
            step_started_at: Instant::now(),
            error: None,
            step_history: vec![StepRecord {
                name: config.initial_step,
                started_at_ms: epoch_ms,
                finished_at_ms: None,
                outcome: StepOutcome::Running,
                agent_id: None,
                agent_name: None,
            }],
            actions: ActionTracker::default(),
            cancelling: false,
            failing: false,
            suspending: false,
            total_retries: 0,
            step_visits: HashMap::new(),
            cron_name: config.cron_name,
            last_nudge_at: None,
        }
    }

    /// Directory where the job's agent executes.
    ///
    /// Uses the workspace path if the job has a managed worktree,
    /// otherwise falls back to the original working directory.
    pub fn execution_dir(&self) -> &std::path::Path {
        self.workspace_path.as_deref().unwrap_or(&self.cwd)
    }

    /// Finalize the most recent step record
    pub fn finalize_current_step(&mut self, outcome: StepOutcome, epoch_ms: u64) {
        if let Some(record) = self.step_history.last_mut() {
            if record.finished_at_ms.is_none() {
                record.finished_at_ms = Some(epoch_ms);
                record.outcome = outcome;
            }
        }
    }

    /// Update the outcome of the most recent step record (without finalizing)
    pub fn update_current_step_outcome(&mut self, outcome: StepOutcome) {
        if let Some(record) = self.step_history.last_mut() {
            if record.finished_at_ms.is_none() {
                record.outcome = outcome;
            }
        }
    }

    /// Push a new step record
    pub fn push_step(&mut self, name: &str, epoch_ms: u64) {
        self.step_history.push(StepRecord {
            name: name.to_string(),
            started_at_ms: epoch_ms,
            finished_at_ms: None,
            outcome: StepOutcome::Running,
            agent_id: None,
            agent_name: None,
        });
    }

    /// Set the agent_id on the most recent step record (if it's still running).
    pub fn set_current_step_agent_id(&mut self, agent_id: &str) {
        if let Some(record) = self.step_history.last_mut() {
            if record.finished_at_ms.is_none() {
                record.agent_id = Some(agent_id.to_string());
            }
        }
    }

    /// Set the agent_name on the most recent step record (if it's still running).
    pub fn set_current_step_agent_name(&mut self, agent_name: &str) {
        if let Some(record) = self.step_history.last_mut() {
            if record.finished_at_ms.is_none() {
                record.agent_name = Some(agent_name.to_string());
            }
        }
    }

    /// Check if the job is in a terminal state
    pub fn is_terminal(&self) -> bool {
        self.step == "done"
            || self.step == "failed"
            || self.step == "cancelled"
            || self.step == "suspended"
    }

    /// Check if the job is suspended (terminal but resumable, never pruned)
    pub fn is_suspended(&self) -> bool {
        self.step == "suspended"
    }

    /// Set the workspace ID and path
    #[cfg(test)]
    pub fn with_workspace(mut self, id: WorkspaceId, path: PathBuf) -> Self {
        self.workspace_id = Some(id);
        self.workspace_path = Some(path);
        self
    }

    /// Increment and return the new attempt count for a given action.
    /// Also tracks cumulative retries (when attempt count > 1).
    pub fn increment_attempt(&mut self, trigger: &str, chain_pos: usize) -> u32 {
        let count = self.actions.increment_attempt(trigger, chain_pos);
        if count > 1 {
            self.total_retries += 1;
        }
        count
    }

    /// Record a visit to a step. Returns the new visit count.
    pub fn record_step_visit(&mut self, step: &str) -> u32 {
        let count = self.step_visits.entry(step.to_string()).or_insert(0);
        *count += 1;
        *count
    }

    /// Get the number of times a step has been visited.
    pub fn get_step_visits(&self, step: &str) -> u32 {
        self.step_visits.get(step).copied().unwrap_or(0)
    }
}

crate::builder! {
    pub struct JobBuilder => Job {
        into {
            id: String = "test-1",
            name: String = "test-job",
            kind: String = "build",
            project: String = "",
            step: String = "execute",
            runbook_hash: String = "testhash",
            cwd: PathBuf = "/tmp/test",
        }
        set {
            step_status: StepStatus = StepStatus::Running,
            step_history: Vec<StepRecord> = Vec::new(),
            vars: HashMap<String, String> = HashMap::new(),
            actions: ActionTracker = ActionTracker::default(),
            cancelling: bool = false,
            failing: bool = false,
            suspending: bool = false,
            total_retries: u32 = 0,
            step_visits: HashMap<String, u32> = HashMap::new(),
            last_nudge_at: Option<u64> = None,
        }
        option {
            workspace_id: WorkspaceId = None,
            workspace_path: PathBuf = None,
            error: String = None,
            cron_name: String = None,
        }
        computed {
            step_started_at: Instant = Instant::now(),
            created_at: Instant = Instant::now(),
        }
    }
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
