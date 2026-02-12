// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! First-class agent record for unified tracking in MaterializedState.
//!
//! `AgentRecord` provides a unified view of ALL agents regardless of how they were spawned.
//! It serves as a lookup index that is populated from existing events during WAL replay.

use crate::owner::OwnerId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A first-class agent entity tracked in MaterializedState.
///
/// Provides a unified view of ALL agents regardless of how they were spawned
/// (job-embedded or standalone). Replaces the dual-tracking approach where job
/// agents were implicit in Job.step_history and standalone agents were explicit
/// in Crew.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    /// Agent instance UUID (same as session-id passed to claude)
    pub agent_id: String,
    /// Agent definition name from the runbook
    pub agent_name: String,
    /// Owner of this agent (job or crew)
    pub owner: OwnerId,
    /// Project project
    pub project: String,
    /// Workspace path where the crew
    pub workspace_path: PathBuf,
    /// Current status
    pub status: AgentRecordStatus,
    /// Which adapter runtime manages this agent (local, docker, k8s).
    ///
    /// Set from `AgentSpawned` events. Defaults to `Local` for records
    /// created before this field existed.
    #[serde(default)]
    pub runtime: AgentRuntime,
    /// Bearer token for remote (Docker/K8s) agents.
    ///
    /// Persisted from `AgentSpawned` so reconnect after daemon restart can
    /// re-establish communication without reading the token from the container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// Epoch milliseconds when created
    pub created_at_ms: u64,
    /// Epoch milliseconds of last update
    pub updated_at_ms: u64,
}

/// Which adapter runtime manages an agent.
///
/// Persisted in [`AgentRecord`] so reconciliation after daemon restart knows
/// which adapter to try first without probing all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntime {
    #[default]
    Local,
    Docker,
    Kubernetes,
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

crate::simple_display! {
    AgentRecordStatus {
        Starting => "starting",
        Running => "running",
        Idle => "idle",
        Exited => "exited",
        Gone => "gone",
    }
}
