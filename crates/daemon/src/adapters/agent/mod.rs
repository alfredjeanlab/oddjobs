// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent management adapters
//!
//! This module provides an abstraction layer for managing AI agents (like Claude).
//! The `AgentAdapter` trait encapsulates all agent-specific logic including:
//! - Workspace preparation
//! - Background state monitoring via coop sidecar
//! - Session transcript retrieval
//!
//! # ID Hierarchy
//!
//! ```text
//! workspace_id  - Git worktree path (may outlive job)
//!      │
//!      └── agent_id  - Agent instance (UUID)
//!
//! job_id  - Job execution (references workspace)
//! ```

pub mod attach_proxy;
pub(crate) mod coop;
pub(crate) mod docker;
pub(crate) mod k8s;
pub mod log_entry;
pub(crate) mod remote;
mod router;

pub use coop::LocalAdapter;
pub use router::{CoopInfo, RuntimeRouter};

/// Configuration for reconnecting to an existing agent session
#[derive(Debug, Clone)]
pub struct AgentReconnectConfig {
    pub agent_id: AgentId,
    /// Owner of this agent (job or crew)
    pub owner: OwnerId,
    /// Hint from persisted state about which adapter manages this agent.
    ///
    /// Used by `RuntimeRouter::reconnect` to try the correct adapter first,
    /// falling back to probing other adapters if the hinted one fails.
    pub runtime_hint: AgentRuntime,
    /// Auth token persisted from spawn time.
    ///
    /// When available, remote adapters use this instead of shelling out to
    /// `kubectl exec` or `docker exec` to read the token from the container.
    pub auth_token: Option<String>,
}

// Test support - only compiled for tests or when explicitly requested
#[cfg(test)]
mod fake;
#[cfg(test)]
pub use fake::{AgentCall, FakeAgentAdapter};

use async_trait::async_trait;
use oj_core::{AgentId, AgentRuntime, AgentState, Event, OwnerId};
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
    #[error("session error: {0}")]
    SessionError(String),
    #[error("workspace error: {0}")]
    WorkspaceError(String),
}

/// Configuration for spawning a new agent
#[derive(Debug, Clone)]
pub struct AgentConfig {
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
    pub project_path: PathBuf,
    /// Owner of this agent (job or crew)
    pub owner: OwnerId,
    /// Whether to resume a previous session (coop handles session discovery)
    pub resume: bool,
    /// Container config — when present, route to the container adapter.
    pub container: Option<oj_core::ContainerConfig>,
    /// Git remote URL resolved at job creation time (avoids needing local checkout in pods)
    pub repo: Option<String>,
    /// Git branch resolved at job creation time
    pub branch: Option<String>,
}

impl AgentConfig {
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
            project_path: workspace_path,
            resume: false,
            container: None,
            repo: None,
            branch: None,
        }
    }

    oj_core::setters! {
        into {
            agent_name: String,
            prompt: String,
            job_name: String,
            job_id: String,
        }
        set {
            env: Vec<(String, String)>,
            unset_env: Vec<String>,
            project_path: PathBuf,
        }
        option {
            cwd: PathBuf,
            repo: String,
            branch: String,
        }
    }
}

/// Handle to a running agent
#[derive(Debug, Clone)]
pub struct AgentHandle {
    /// Public agent identifier
    pub agent_id: AgentId,
    /// Which adapter runtime manages this agent.
    ///
    /// Set by the `RuntimeRouter` based on which adapter was used for spawn.
    /// Individual adapters default to `Local`; the router overrides as needed.
    pub runtime: oj_core::AgentRuntime,
    /// Auth token generated at spawn time (for remote agents).
    ///
    /// Persisted through the WAL so reconnect after daemon restart can
    /// re-establish communication without shelling out to `kubectl exec` or
    /// `docker exec` to read the token from the running container.
    pub auth_token: Option<String>,
}

impl AgentHandle {
    /// Create a new agent handle
    pub fn new(agent_id: AgentId) -> Self {
        Self { agent_id, runtime: oj_core::AgentRuntime::Local, auth_token: None }
    }
}

/// Adapter for managing AI agents
#[async_trait]
pub trait AgentAdapter: Send + Sync + 'static {
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
        config: AgentConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError>;

    /// Send input to an agent
    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError>;

    /// Send a structured response to an agent prompt (plan, permission).
    ///
    /// Default implementation falls back to `send` with the text field.
    async fn respond(
        &self,
        agent_id: &AgentId,
        response: &oj_core::PromptResponse,
    ) -> Result<(), AgentAdapterError> {
        self.send(agent_id, response.text.as_deref().unwrap_or("")).await
    }

    /// Kill an agent
    ///
    /// This stops both the agent's session and its background watcher.
    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError>;

    /// Reconnect to an existing agent session (after daemon restart).
    ///
    /// Sets up background monitoring without spawning a new process.
    /// The agent must already be alive.
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

    /// Extract the last assistant text message from the agent's session log.
    ///
    /// Used to provide context on decisions (idle, dead, prompt, question).
    async fn last_message(&self, agent_id: &AgentId) -> Option<String>;

    /// Resolve a pending stop gate, allowing the agent to exit on its next attempt.
    async fn resolve_stop(&self, agent_id: &AgentId);

    /// Check if an agent is alive (session exists and agent process is running).
    ///
    /// Returns `false` if the agent is not found or the check fails.
    async fn is_alive(&self, agent_id: &AgentId) -> bool;

    /// Capture recent output from the agent's terminal.
    ///
    /// Returns the last `lines` lines of visible terminal output.
    async fn capture_output(
        &self,
        agent_id: &AgentId,
        lines: u32,
    ) -> Result<String, AgentAdapterError>;

    /// Fetch the full session transcript from the agent's coop sidecar.
    ///
    /// Returns the complete JSONL transcript content for archival.
    /// Returns an empty string if the transcript is unavailable.
    async fn fetch_transcript(&self, agent_id: &AgentId) -> Result<String, AgentAdapterError>;

    /// Fetch cumulative usage metrics from the agent's sidecar.
    ///
    /// Returns `None` if the agent is unreachable or has no usage data.
    async fn fetch_usage(&self, agent_id: &AgentId) -> Option<UsageData>;

    /// Get coop connection info for agent attach proxying (infrastructure).
    ///
    /// Returns connection information (URL, auth token) for proxying attach
    /// requests to this agent's coop process. Only routers implement this.
    ///
    /// Default: None (not a router, attach not supported)
    fn get_coop_host(&self, _agent_id: &AgentId) -> Option<CoopInfo> {
        None
    }

    /// Check if this adapter only supports remote execution (infrastructure).
    ///
    /// Used to determine workspace adapter selection at startup.
    /// Returns `true` for Kubernetes (no local filesystem).
    ///
    /// Default: false (supports local execution)
    fn is_remote_only(&self) -> bool {
        false
    }
}

/// Cumulative token/cost usage data from an agent session.
#[derive(Debug, Clone, Default)]
pub struct UsageData {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_cost_usd: f64,
    pub total_api_ms: u64,
}

impl UsageData {
    /// Parse usage data from a coop `/api/v1/session/usage` JSON response.
    pub(crate) fn from_json(json: &serde_json::Value) -> Self {
        Self {
            input_tokens: json.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            output_tokens: json.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            cache_read_tokens: json.get("cache_read_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            cache_write_tokens: json
                .get("cache_write_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            total_cost_usd: json.get("total_cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0),
            total_api_ms: json.get("total_api_ms").and_then(|v| v.as_u64()).unwrap_or(0),
        }
    }
}

/// Map a coop agent state JSON response to an `AgentState`.
///
/// Shared by all three adapters (coop, docker, k8s) since they all talk to
/// the same coop HTTP API.
pub(crate) fn parse_coop_agent_state(json: &serde_json::Value) -> AgentState {
    let state_str = json.get("state").and_then(|v| v.as_str()).unwrap_or("unknown");
    match state_str {
        "starting" | "working" => AgentState::Working,
        "idle" | "prompt" => AgentState::WaitingForInput,
        "error" => {
            let detail =
                json.get("error_detail").and_then(|v| v.as_str()).unwrap_or("unknown error");
            let category = json.get("error_category").and_then(|v| v.as_str());
            let error = match category {
                Some("Unauthorized") => oj_core::AgentError::Unauthorized,
                Some("OutOfCredits") => oj_core::AgentError::OutOfCredits,
                _ => oj_core::AgentError::Other(detail.to_string()),
            };
            AgentState::Failed(error)
        }
        "exited" => AgentState::SessionGone,
        _ => AgentState::Working,
    }
}

/// Add `--allow-dangerously-skip-permissions` if skip-permissions is present.
pub(crate) fn augment_command_for_skip_permissions(command: &str) -> String {
    if command.contains("--dangerously-skip-permissions")
        && !command.contains("--allow-dangerously-skip-permissions")
    {
        format!("{} --allow-dangerously-skip-permissions", command)
    } else {
        command.to_string()
    }
}

/// Generate a random bearer token for per-agent auth.
pub(crate) fn generate_auth_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Detect the git remote URL from a project directory.
pub(crate) async fn detect_git_remote(project_path: &std::path::Path) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(project_path)
        .output()
        .await
        .ok()?;
    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !url.is_empty() {
            return Some(url);
        }
    }
    None
}

/// Detect the current git branch of a workspace directory (blocking variant).
///
/// Uses `std::process::Command` — suitable for contexts where the workspace
/// path may not exist yet and blocking is acceptable (e.g., K8s pod spec build).
pub(crate) fn detect_git_branch_blocking(workspace_path: &std::path::Path) -> Option<String> {
    if !workspace_path.exists() {
        return None;
    }
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace_path)
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() && branch != "HEAD" {
            return Some(branch);
        }
    }
    None
}

/// Detect the current git branch of a workspace directory (async variant).
pub(crate) async fn detect_git_branch_async(workspace_path: &std::path::Path) -> Option<String> {
    if !workspace_path.exists() {
        return None;
    }
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace_path)
        .output()
        .await
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !branch.is_empty() && branch != "HEAD" {
            return Some(branch);
        }
    }
    None
}

/// Poll a remote coop health endpoint until it responds successfully.
pub(crate) async fn poll_until_ready(
    addr: &str,
    auth_token: &str,
    poll_ms: u64,
    max_attempts: usize,
    label: &str,
) -> Result<(), AgentAdapterError> {
    for i in 0..max_attempts {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;
        }
        if docker::http::get_authed(addr, "/api/v1/health", auth_token).await.is_ok() {
            tracing::info!(%addr, attempt = i, "{} coop health check succeeded", label);
            return Ok(());
        }
    }
    Err(AgentAdapterError::SpawnFailed(format!(
        "coop failed to become ready within {}s ({})",
        (max_attempts as u64 * poll_ms) / 1000,
        label,
    )))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
