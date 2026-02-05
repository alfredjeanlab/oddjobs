// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Effects represent side effects the system needs to perform

use crate::agent::AgentId;
use crate::agent_run::AgentRunId;
use crate::event::Event;
use crate::job::JobId;
use crate::session::SessionId;
use crate::timer::TimerId;
use crate::workspace::WorkspaceId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Effects that need to be executed by the runtime
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    // === Event emission ===
    /// Emit an event into the system event bus
    Emit { event: Event },

    // === Agent-level effects (preferred for job operations) ===
    /// Spawn a new agent
    SpawnAgent {
        agent_id: AgentId,
        agent_name: String,
        job_id: JobId,
        /// For standalone agents, the AgentRunId that owns this spawn
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_run_id: Option<AgentRunId>,
        workspace_path: PathBuf,
        input: HashMap<String, String>,
        /// Command to execute (already interpolated)
        command: String,
        /// Environment variables
        env: Vec<(String, String)>,
        /// Working directory override
        cwd: Option<PathBuf>,
        /// Adapter-specific session configuration (provider -> config as JSON)
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        session_config: HashMap<String, serde_json::Value>,
    },

    /// Send input to an agent
    SendToAgent { agent_id: AgentId, input: String },

    /// Kill an agent
    KillAgent { agent_id: AgentId },

    // === Session-level effects (low-level, used by AgentAdapter) ===
    /// Send input to an existing session (low-level)
    SendToSession {
        session_id: SessionId,
        input: String,
    },

    /// Kill a session (low-level)
    KillSession { session_id: SessionId },

    // === Workspace effects ===
    /// Create a managed workspace (creates directory and tracks lifecycle)
    CreateWorkspace {
        workspace_id: WorkspaceId,
        path: PathBuf,
        owner: Option<String>,
        /// "folder" or "worktree" (replaces old "mode" field)
        #[serde(default, alias = "mode")]
        workspace_type: Option<String>,
        /// For worktree: the repo root to create the worktree from
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo_root: Option<PathBuf>,
        /// For worktree: the branch name to create
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// For worktree: the start point (commit/branch to base from)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_point: Option<String>,
    },

    /// Delete a managed workspace (removes directory and cleans up)
    DeleteWorkspace { workspace_id: WorkspaceId },

    // === Timer effects ===
    /// Set a timer
    SetTimer {
        id: TimerId,
        #[serde(with = "duration_serde")]
        duration: Duration,
    },

    /// Cancel a timer
    CancelTimer { id: TimerId },

    // === Shell effects ===
    /// Execute a shell command
    Shell {
        /// Job this belongs to
        job_id: JobId,
        /// Step name
        step: String,
        /// Command to execute (already interpolated)
        command: String,
        /// Working directory
        cwd: PathBuf,
        /// Environment variables
        env: HashMap<String, String>,
    },

    // === Worker effects ===
    /// Run the queue's list command to get available items
    PollQueue {
        worker_name: String,
        list_command: String,
        cwd: PathBuf,
    },

    /// Run the queue's take command to claim an item
    TakeQueueItem {
        worker_name: String,
        take_command: String,
        cwd: PathBuf,
        /// ID of the item being taken (passed through to the completion event)
        item_id: String,
        /// Full item data (passed through to the completion event for job creation)
        item: serde_json::Value,
    },

    /// Report queue item completion/failure to external system
    ReportQueueItem {
        worker_name: String,
        command: String,
        cwd: PathBuf,
        item_id: String,
        /// true for on_done, false for on_fail
        is_completion: bool,
    },

    // === Notification effects ===
    /// Send a desktop notification
    Notify {
        /// Notification title
        title: String,
        /// Notification message body
        message: String,
    },
}

impl Effect {
    /// Effect name for log spans (e.g., "spawn_agent", "shell")
    pub fn name(&self) -> &'static str {
        match self {
            Effect::Emit { .. } => "emit",
            Effect::SpawnAgent { .. } => "spawn_agent",
            Effect::SendToAgent { .. } => "send_to_agent",
            Effect::KillAgent { .. } => "kill_agent",
            Effect::SendToSession { .. } => "send_to_session",
            Effect::KillSession { .. } => "kill_session",
            Effect::CreateWorkspace { .. } => "create_workspace",
            Effect::DeleteWorkspace { .. } => "delete_workspace",
            Effect::SetTimer { .. } => "set_timer",
            Effect::CancelTimer { .. } => "cancel_timer",
            Effect::Shell { .. } => "shell",
            Effect::PollQueue { .. } => "poll_queue",
            Effect::TakeQueueItem { .. } => "take_queue_item",
            Effect::ReportQueueItem { .. } => "report_queue_item",
            Effect::Notify { .. } => "notify",
        }
    }

    /// Key-value pairs for structured logging
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        match self {
            Effect::Emit { event } => {
                vec![("event", event.log_summary())]
            }
            Effect::SpawnAgent {
                agent_id,
                agent_name,
                job_id,
                agent_run_id,
                workspace_path,
                command,
                cwd,
                ..
            } => {
                let mut fields = vec![
                    ("agent_id", agent_id.to_string()),
                    ("agent_name", agent_name.clone()),
                    ("job_id", job_id.to_string()),
                    ("workspace_path", workspace_path.display().to_string()),
                    ("command", command.clone()),
                    (
                        "cwd",
                        cwd.as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                    ),
                ];
                if let Some(ref run_id) = agent_run_id {
                    fields.push(("agent_run_id", run_id.to_string()));
                }
                fields
            }
            Effect::SendToAgent { agent_id, .. } => vec![("agent_id", agent_id.to_string())],
            Effect::KillAgent { agent_id } => vec![("agent_id", agent_id.to_string())],
            Effect::SendToSession { session_id, .. } => {
                vec![("session_id", session_id.to_string())]
            }
            Effect::KillSession { session_id } => vec![("session_id", session_id.to_string())],
            Effect::CreateWorkspace {
                workspace_id, path, ..
            } => vec![
                ("workspace_id", workspace_id.to_string()),
                ("path", path.display().to_string()),
            ],
            Effect::DeleteWorkspace { workspace_id } => {
                vec![("workspace_id", workspace_id.to_string())]
            }
            Effect::SetTimer { id, duration } => vec![
                ("timer_id", id.to_string()),
                ("duration_ms", duration.as_millis().to_string()),
            ],
            Effect::CancelTimer { id } => vec![("timer_id", id.to_string())],
            Effect::Shell {
                job_id, step, cwd, ..
            } => vec![
                ("job_id", job_id.to_string()),
                ("step", step.clone()),
                ("cwd", cwd.display().to_string()),
            ],
            Effect::PollQueue {
                worker_name, cwd, ..
            } => vec![
                ("worker_name", worker_name.clone()),
                ("cwd", cwd.display().to_string()),
            ],
            Effect::TakeQueueItem {
                worker_name,
                cwd,
                item_id,
                ..
            } => vec![
                ("worker_name", worker_name.clone()),
                ("cwd", cwd.display().to_string()),
                ("item_id", item_id.clone()),
            ],
            Effect::ReportQueueItem {
                worker_name,
                cwd,
                item_id,
                is_completion,
                ..
            } => vec![
                ("worker_name", worker_name.clone()),
                ("cwd", cwd.display().to_string()),
                ("item_id", item_id.clone()),
                ("is_completion", is_completion.to_string()),
            ],
            Effect::Notify { title, .. } => vec![("title", title.clone())],
        }
    }
}

mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
        duration.as_millis().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(d)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
#[path = "effect_tests.rs"]
mod tests;
