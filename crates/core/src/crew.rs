// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Crew entity.
//!
//! An `Crew` represents a standalone agent invocation triggered by a
//! `command { run = { agent = "..." } }` block. Unlike job-embedded agents,
//! standalone agents are top-level WAL entities with self-resolving lifecycle.

use crate::actions::ActionTracker;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

crate::define_id! {
    /// Unique identifier for a crew.
    pub struct CrewId;
}

/// Status of a crew.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrewStatus {
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

impl CrewStatus {
    /// Whether this status is terminal (no further transitions expected)
    pub fn is_terminal(&self) -> bool {
        matches!(self, CrewStatus::Completed | CrewStatus::Failed)
    }
}

crate::simple_display! {
    CrewStatus {
        Starting => "starting",
        Running => "running",
        Waiting => "waiting",
        Completed => "completed",
        Failed => "failed",
        Escalated => "escalated",
    }
}

/// A crew instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Crew {
    pub id: String,
    /// Agent definition name from the runbook
    pub agent_name: String,
    /// Command that triggered this run
    pub command_name: String,
    /// Project project
    pub project: String,
    /// Directory where the crew
    pub cwd: PathBuf,
    /// Runbook content hash for cache lookup
    pub runbook_hash: String,
    /// Current status
    pub status: CrewStatus,
    /// UUID of the spawned agent (set on start)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Error message if failed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Epoch milliseconds when created
    pub created_at_ms: u64,
    /// Epoch milliseconds of last update
    pub updated_at_ms: u64,
    /// Action attempt tracking and agent signal state.
    #[serde(flatten)]
    pub actions: ActionTracker,
    /// Variables passed to the command
    #[serde(default)]
    pub vars: HashMap<String, String>,
    /// Epoch milliseconds when the last nudge was sent.
    /// Used to suppress auto-resume from our own nudge text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_nudge_at: Option<u64>,
}

crate::builder! {
    pub struct CrewBuilder => Crew {
        into {
            id: String = "run-1",
            agent_name: String = "worker",
            command_name: String = "agent_cmd",
            project: String = "",
            cwd: PathBuf = "/tmp/test",
            runbook_hash: String = "testhash",
        }
        set {
            status: CrewStatus = CrewStatus::Running,
            created_at_ms: u64 = 0,
            updated_at_ms: u64 = 0,
            actions: ActionTracker = ActionTracker::default(),
            vars: HashMap<String, String> = HashMap::new(),
            last_nudge_at: Option<u64> = None,
        }
        option {
            agent_id: String = Some("agent-uuid-1".to_string()),
            error: String = None,
        }
    }
}

#[cfg(test)]
#[path = "crew_tests.rs"]
mod tests;
