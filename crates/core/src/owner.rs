// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Owner identification for agent events.
//!
//! Agents can be owned by either a Job (job-embedded agents) or an AgentRun
//! (standalone agents). This module provides a tagged union type to represent
//! that ownership, enabling proper routing during WAL replay.

use crate::agent_run::AgentRunId;
use crate::job::JobId;
use serde::{Deserialize, Serialize};

/// Owner of an agent event.
///
/// Used to route agent state events (Working, Waiting, Failed, Exited, Gone)
/// to the correct entity during WAL replay.
///
/// Serializes as a tagged enum:
/// - `{"type": "job", "id": "job-123"}`
/// - `{"type": "agent_run", "id": "ar-456"}`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id")]
pub enum OwnerId {
    /// Agent is owned by a job (job-embedded agent)
    #[serde(rename = "job")]
    Job(JobId),
    /// Agent is owned by an agent run (standalone agent)
    #[serde(rename = "agent_run")]
    AgentRun(AgentRunId),
}

impl OwnerId {
    /// Create a Job owner.
    pub fn job(id: JobId) -> Self {
        OwnerId::Job(id)
    }

    /// Create an AgentRun owner.
    pub fn agent_run(id: AgentRunId) -> Self {
        OwnerId::AgentRun(id)
    }
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
