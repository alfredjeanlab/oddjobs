// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! First-class agent record for unified tracking in MaterializedState.
//!
//! `AgentRecord` provides a unified view of ALL agents regardless of how they
//! were spawned (job-embedded or standalone). It serves as a lookup index that
//! is populated from existing events during WAL replay — no new event types
//! are needed.

use crate::owner::OwnerId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// A first-class agent entity tracked in MaterializedState.
///
/// Provides a unified view of ALL agents regardless of how they were spawned
/// (job-embedded or standalone). Replaces the dual-tracking approach where job
/// agents were implicit in Job.step_history and standalone agents were explicit
/// in AgentRun.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    /// Agent instance UUID (same as session-id passed to claude)
    pub agent_id: String,
    /// Agent definition name from the runbook
    pub agent_name: String,
    /// Owner of this agent (job or agent_run)
    pub owner: OwnerId,
    /// Project namespace
    pub namespace: String,
    /// Workspace path where the agent runs
    pub workspace_path: PathBuf,
    /// tmux session ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Current status
    pub status: AgentRecordStatus,
    /// Epoch milliseconds when created
    pub created_at_ms: u64,
    /// Epoch milliseconds of last update
    pub updated_at_ms: u64,
}

/// Status of an agent in the unified agent record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRecordStatus {
    /// Agent is being spawned
    Starting,
    /// Agent is actively working
    Running,
    /// Agent is idle / waiting for input
    Idle,
    /// Agent process has exited
    Exited,
    /// Agent session is gone (unexpected termination)
    Gone,
}

impl fmt::Display for AgentRecordStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Idle => write!(f, "idle"),
            Self::Exited => write!(f, "exited"),
            Self::Gone => write!(f, "gone"),
        }
    }
}

#[cfg(test)]
#[path = "agent_record_tests.rs"]
mod tests;
