// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Standalone agent run entity.
//!
//! An `AgentRun` represents a standalone agent invocation triggered by a
//! `command { run = { agent = "..." } }` block. Unlike pipeline-embedded agents,
//! standalone agents are top-level WAL entities with self-resolving lifecycle.

use crate::pipeline::AgentSignal;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

/// Unique identifier for a standalone agent run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentRunId(pub String);

impl AgentRunId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentRunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for AgentRunId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for AgentRunId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl PartialEq<str> for AgentRunId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AgentRunId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl Borrow<str> for AgentRunId {
    fn borrow(&self) -> &str {
        &self.0
    }
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
    /// Action attempt tracking (same pattern as Pipeline)
    #[serde(default)]
    pub action_attempts: HashMap<String, u32>,
    /// Signal from agent indicating completion intent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_signal: Option<AgentSignal>,
    /// Variables passed to the command
    #[serde(default)]
    pub vars: HashMap<String, String>,
}

/// Build the string key for action_attempts: "trigger:chain_pos".
fn action_key(trigger: &str, chain_pos: usize) -> String {
    format!("{trigger}:{chain_pos}")
}

impl AgentRun {
    /// Check if the agent run is in a terminal state
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// Increment and return the new attempt count for a given action
    pub fn increment_action_attempt(&mut self, trigger: &str, chain_pos: usize) -> u32 {
        let key = action_key(trigger, chain_pos);
        let count = self.action_attempts.entry(key).or_insert(0);
        *count += 1;
        *count
    }

    /// Reset action attempts
    pub fn reset_action_attempts(&mut self) {
        self.action_attempts.clear();
    }

    /// Clear agent signal
    pub fn clear_agent_signal(&mut self) {
        self.agent_signal = None;
    }
}

#[cfg(test)]
#[path = "agent_run_tests.rs"]
mod tests;
