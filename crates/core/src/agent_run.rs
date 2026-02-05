// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Standalone agent run entity.
//!
//! An `AgentRun` represents a standalone agent invocation triggered by a
//! `command { run = { agent = "..." } }` block. Unlike job-embedded agents,
//! standalone agents are top-level WAL entities with self-resolving lifecycle.

use crate::action_tracker::ActionTracker;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

crate::define_id! {
    /// Unique identifier for a standalone agent run.
    pub struct AgentRunId;
}

/// Status of a standalone agent run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    /// Agent is being spawned
    Starting,
    /// Agent is actively working
    Running,
    /// Waiting for human intervention (escalated)
    Waiting,
    /// Agent completed successfully
    Completed,
    /// Agent failed
    Failed,
    /// Agent escalated to human
    Escalated,
}

impl AgentRunStatus {
    /// Whether this status is terminal (no further transitions expected)
    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentRunStatus::Completed | AgentRunStatus::Failed)
    }
}

impl fmt::Display for AgentRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentRunStatus::Starting => write!(f, "starting"),
            AgentRunStatus::Running => write!(f, "running"),
            AgentRunStatus::Waiting => write!(f, "waiting"),
            AgentRunStatus::Completed => write!(f, "completed"),
            AgentRunStatus::Failed => write!(f, "failed"),
            AgentRunStatus::Escalated => write!(f, "escalated"),
        }
    }
}

/// A standalone agent run instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: String,
    /// Agent definition name from the runbook
    pub agent_name: String,
    /// Command that triggered this run
    pub command_name: String,
    /// Project namespace
    pub namespace: String,
    /// Directory where the agent runs
    pub cwd: PathBuf,
    /// Runbook content hash for cache lookup
    pub runbook_hash: String,
    /// Current status
    pub status: AgentRunStatus,
    /// UUID of the spawned agent (set on start)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// tmux session ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Error message if failed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Epoch milliseconds when created
    pub created_at_ms: u64,
    /// Epoch milliseconds of last update
    pub updated_at_ms: u64,
    /// Action attempt tracking and agent signal state.
    #[serde(flatten)]
    pub action_tracker: ActionTracker,
    /// Variables passed to the command
    #[serde(default)]
    pub vars: HashMap<String, String>,
    /// Session log file size when idle grace timer was set.
    /// Used to detect activity during the grace period.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_grace_log_size: Option<u64>,
    /// Epoch milliseconds when the last nudge was sent.
    /// Used to suppress auto-resume from our own nudge text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_nudge_at: Option<u64>,
}

impl AgentRun {
    /// Check if the agent run is in a terminal state
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// Increment and return the new attempt count for a given action
    pub fn increment_action_attempt(&mut self, trigger: &str, chain_pos: usize) -> u32 {
        self.action_tracker
            .increment_action_attempt(trigger, chain_pos)
    }

    /// Reset action attempts
    pub fn reset_action_attempts(&mut self) {
        self.action_tracker.reset_action_attempts();
    }

    /// Clear agent signal
    pub fn clear_agent_signal(&mut self) {
        self.action_tracker.clear_agent_signal();
    }
}

#[cfg(test)]
#[path = "agent_run_tests.rs"]
mod tests;
