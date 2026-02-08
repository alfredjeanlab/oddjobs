// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent management adapters
//!
//! This module provides an abstraction layer for managing AI agents (like Claude).
//! The `AgentAdapter` trait encapsulates all agent-specific logic including:
//! - Workspace preparation
//! - Session log parsing
//! - Background monitoring via file watchers
//!
//! # ID Hierarchy
//!
//! ```text
//! workspace_id  - Git worktree path (may outlive job)
//!      │
//!      └── agent_id  - Agent instance (UUID, used as --session-id for claude)
//!               │
//!               └── session_id  - Internal to AgentAdapter (hidden)
//!
//! job_id  - Job execution (references workspace)
//! ```

mod claude;
pub mod log_entry;
mod watcher;

pub use claude::{extract_process_name, ClaudeAgentAdapter};
pub use watcher::{extract_last_assistant_text, find_session_log};

/// Configuration for reconnecting to an existing agent session
#[derive(Debug, Clone)]
pub struct AgentReconnectConfig {
    pub agent_id: AgentId,
    pub session_id: String,
    pub workspace_path: PathBuf,
    pub process_name: String,
    /// Owner of this agent (job or agent_run)
    pub owner: OwnerId,
}

// Test support - only compiled for tests or when explicitly requested
#[cfg(any(test, feature = "test-support"))]
mod fake;
#[cfg(any(test, feature = "test-support"))]
pub use fake::{AgentCall, FakeAgentAdapter};

use async_trait::async_trait;
use oj_core::{AgentId, AgentState, Event, OwnerId};
use std::path::PathBuf;
use thiserror::Error;
use tokio::sync::mpsc;

/// Errors from agent adapter operations
#[derive(Debug, Error)]
pub enum AgentAdapterError {
    #[error("agent not found: {0}")]
    NotFound(String),
    #[error("spawn failed: {0}")]
    SpawnFailed(String),
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("kill failed: {0}")]
    KillFailed(String),
    #[error("session error: {0}")]
    SessionError(String),
    #[error("workspace error: {0}")]
    WorkspaceError(String),
}

/// Configuration for spawning a new agent
#[derive(Debug, Clone)]
pub struct AgentSpawnConfig {
    /// Unique identifier for this agent instance
    pub agent_id: AgentId,
    /// Name of the agent (e.g., "claude")
    pub agent_name: String,
    /// Command to execute
    pub command: String,
    /// Environment variables
    pub env: Vec<(String, String)>,
    /// Environment variables to explicitly unset in the spawned session
    pub unset_env: Vec<String>,
    /// Path to the workspace directory
    pub workspace_path: PathBuf,
    /// Optional working directory override
    pub cwd: Option<PathBuf>,
    /// Initial prompt for the agent
    pub prompt: String,
    /// Name of the job
    pub job_name: String,
    /// Job ID
    pub job_id: String,
    /// Root of the project
    pub project_root: PathBuf,
    /// Adapter-specific session configuration (provider -> config as JSON)
    pub session_config: std::collections::HashMap<String, serde_json::Value>,
    /// Owner of this agent (job or agent_run)
    pub owner: OwnerId,
}

impl AgentSpawnConfig {
    pub fn new(
        agent_id: AgentId,
        command: impl Into<String>,
        workspace_path: PathBuf,
        owner: OwnerId,
    ) -> Self {
        Self {
            agent_id,
            command: command.into(),
            workspace_path: workspace_path.clone(),
            owner,
            agent_name: String::new(),
            env: Vec::new(),
            unset_env: Vec::new(),
            cwd: None,
            prompt: String::new(),
            job_name: String::new(),
            job_id: String::new(),
            project_root: workspace_path,
            session_config: std::collections::HashMap::new(),
        }
    }

    pub fn agent_name(mut self, v: impl Into<String>) -> Self {
        self.agent_name = v.into();
        self
    }
    pub fn prompt(mut self, v: impl Into<String>) -> Self {
        self.prompt = v.into();
        self
    }
    pub fn job_name(mut self, v: impl Into<String>) -> Self {
        self.job_name = v.into();
        self
    }
    pub fn job_id(mut self, v: impl Into<String>) -> Self {
        self.job_id = v.into();
        self
    }
    pub fn env(mut self, v: Vec<(String, String)>) -> Self {
        self.env = v;
        self
    }
    pub fn unset_env(mut self, v: Vec<String>) -> Self {
        self.unset_env = v;
        self
    }
    pub fn cwd(mut self, v: PathBuf) -> Self {
        self.cwd = Some(v);
        self
    }
    pub fn project_root(mut self, v: PathBuf) -> Self {
        self.project_root = v;
        self
    }
    pub fn session_config(
        mut self,
        v: std::collections::HashMap<String, serde_json::Value>,
    ) -> Self {
        self.session_config = v;
        self
    }
}

/// Handle to a running agent
#[derive(Debug, Clone)]
pub struct AgentHandle {
    /// Public agent identifier
    pub agent_id: AgentId,
    /// Session identifier (assigned by the adapter)
    pub session_id: String,
    /// Path to the agent's workspace
    pub workspace_path: PathBuf,
}

impl AgentHandle {
    /// Create a new agent handle
    pub fn new(agent_id: AgentId, session_id: String, workspace_path: PathBuf) -> Self {
        Self {
            agent_id,
            session_id,
            workspace_path,
        }
    }
}

/// Adapter for managing AI agents
#[async_trait]
pub trait AgentAdapter: Clone + Send + Sync + 'static {
    /// Spawn a new agent
    ///
    /// This method:
    /// 1. Prepares the workspace (creates CLAUDE.md, etc.)
    /// 2. Spawns the underlying session
    /// 3. Starts a background watcher that emits events
    ///
    /// The `event_tx` channel receives `AgentStateChanged` events as the agent's
    /// state changes (detected via file watching).
    async fn spawn(
        &self,
        config: AgentSpawnConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError>;

    /// Send input to an agent
    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError>;

    /// Kill an agent
    ///
    /// This stops both the agent's session and its background watcher.
    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError>;

    /// Reconnect to an existing agent session (after daemon restart).
    ///
    /// Sets up background monitoring without spawning a new session.
    /// The session must already be alive in tmux.
    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError>;

    /// Get the current state of an agent
    ///
    /// This is a point-in-time check; for continuous monitoring, use the
    /// event channel from `spawn()`.
    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError>;

    /// Get the current size of the agent's session log file in bytes.
    ///
    /// Returns `None` if the log file doesn't exist or can't be read.
    /// Used by the idle grace timer to detect activity during the grace period.
    async fn session_log_size(&self, agent_id: &AgentId) -> Option<u64>;

    /// Extract the last assistant text message from the agent's session log.
    ///
    /// Used to provide context on decisions (idle, dead, prompt, question).
    async fn last_assistant_message(&self, agent_id: &AgentId) -> Option<String>;
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
