// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Claude agent adapter implementation

use super::log_entry::AgentLogMessage;
use super::watcher::{parse_session_log, start_watcher, WatcherConfig};
use super::{AgentAdapter, AgentAdapterError, AgentHandle, AgentReconnectConfig, AgentSpawnConfig};
use crate::session::SessionAdapter;
use async_trait::async_trait;
use oj_core::{AgentId, AgentState, Event};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Extract the binary basename from a command string.
///
/// Handles absolute paths (`/usr/bin/claude` → `claude`), relative paths
/// (`./claude` → `claude`), and plain names (`claudeless` → `claudeless`).
/// Falls back to `"claude"` for empty strings.
pub fn extract_process_name(command: &str) -> String {
    command
        .split_whitespace()
        .next()
        .and_then(|first| first.rsplit('/').next())
        .unwrap_or("claude")
        .to_string()
}

/// Generate a friendly tmux session name: `{job}-{step}-{random}`
fn generate_session_name(job_name: &str, step_name: &str) -> String {
    let sanitized_job = sanitize_for_tmux(job_name, 20);
    let sanitized_step = sanitize_for_tmux(step_name, 15);
    let random_suffix = generate_short_random(4);

    format!("{}-{}-{}", sanitized_job, sanitized_step, random_suffix)
}

/// Sanitize a string for tmux session names (replace non-alphanumeric with hyphens).
fn sanitize_for_tmux(s: &str, max_len: usize) -> String {
    let sanitized: String = s
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '-',
        })
        .collect();

    // Collapse multiple hyphens and trim
    let collapsed = sanitized
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    // Truncate to max length (avoid cutting mid-hyphen)
    if collapsed.len() <= max_len {
        collapsed
    } else {
        collapsed[..max_len].trim_end_matches('-').to_string()
    }
}

/// Generate a short random hex string.
fn generate_short_random(len: usize) -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..len)
        .map(|_| format!("{:x}", rng.random::<u8>() % 16))
        .collect()
}

/// Add `--allow-dangerously-skip-permissions` if skip-permissions is present.
fn augment_command_for_skip_permissions(command: &str) -> String {
    if command.contains("--dangerously-skip-permissions")
        && !command.contains("--allow-dangerously-skip-permissions")
    {
        format!("{} --allow-dangerously-skip-permissions", command)
    } else {
        command.to_string()
    }
}

/// Result of polling for an interactive prompt in a session pane.
#[derive(Debug, PartialEq)]
pub(crate) enum PromptResult {
    /// Prompt detected and handled (accepted or detected-only).
    Handled,
    /// No prompt detected within the polling window.
    NotPresent,
    /// Unexpected content (e.g. error output) detected in the pane.
    Unexpected(String),
}

/// Configuration for detecting and handling an interactive prompt.
pub(crate) struct PromptCheck<'a> {
    /// Patterns to match in pane output. Matching logic controlled by `match_any`.
    pub detect: &'a [&'a str],
    /// If true, any pattern triggers; if false, all patterns must be present.
    pub match_any: bool,
    /// Key to send when the prompt is detected (None = detect only).
    pub response: Option<&'a str>,
    /// Whether to treat error output as unexpected (vs. ignoring it).
    pub check_errors: bool,
}

/// Poll a session pane for an interactive prompt and optionally respond.
pub(crate) async fn poll_for_prompt<S: SessionAdapter>(
    sessions: &S,
    session_id: &str,
    max_attempts: usize,
    check: &PromptCheck<'_>,
) -> Result<PromptResult, AgentAdapterError> {
    let interval = Duration::from_millis(200);

    for attempt in 0..max_attempts {
        if attempt > 0 {
            tokio::time::sleep(interval).await;
        }

        let output = match sessions.capture_output(session_id, 50).await {
            Ok(out) => out,
            Err(_) => continue,
        };

        let matched = if check.match_any {
            check.detect.iter().any(|p| output.contains(p))
        } else {
            check.detect.iter().all(|p| output.contains(p))
        };
        if matched {
            if let Some(key) = check.response {
                sessions
                    .send(session_id, key)
                    .await
                    .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))?;
            }
            return Ok(PromptResult::Handled);
        }

        if check.check_errors && (output.contains("Error:") || output.contains("error:")) {
            return Ok(PromptResult::Unexpected(output));
        }
    }

    Ok(PromptResult::NotPresent)
}

/// Log the result of a prompt poll for consistent tracing.
fn log_prompt_result(agent_id: &AgentId, name: &str, result: &PromptResult) {
    match result {
        PromptResult::Handled => tracing::info!(%agent_id, "{} prompt accepted", name),
        PromptResult::NotPresent => tracing::debug!(%agent_id, "no {} prompt detected", name),
        PromptResult::Unexpected(output) => tracing::warn!(
            %agent_id, output = %output,
            "unexpected output while checking for {} prompt", name
        ),
    }
}

/// Agent adapter for Claude Code
#[derive(Clone)]
pub struct ClaudeAgentAdapter<S: SessionAdapter> {
    sessions: S,
    /// Map from agent_id to (session_id, shutdown_tx, workspace_path)
    agents: Arc<Mutex<HashMap<AgentId, AgentInfo>>>,
    /// Channel for sending extracted log entries to the engine
    log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
}

struct AgentInfo {
    session_id: String,
    workspace_path: PathBuf,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl<S: SessionAdapter> ClaudeAgentAdapter<S> {
    /// Create a new Claude agent adapter
    pub fn new(sessions: S) -> Self {
        Self {
            sessions,
            agents: Arc::new(Mutex::new(HashMap::new())),
            log_entry_tx: None,
        }
    }

    /// Create a new Claude agent adapter with agent log extraction
    pub fn with_log_entry_tx(mut self, tx: mpsc::Sender<AgentLogMessage>) -> Self {
        self.log_entry_tx = Some(tx);
        self
    }

    /// Start a file watcher and register the agent in the agent map.
    fn start_watcher_and_register(
        &self,
        config: WatcherConfig,
        workspace_path: PathBuf,
        event_tx: mpsc::Sender<Event>,
    ) -> AgentHandle {
        let agent_id = config.agent_id.clone();
        let session_id = config.tmux_session_id.clone();
        let shutdown_tx = start_watcher(
            config,
            self.sessions.clone(),
            event_tx,
            self.log_entry_tx.clone(),
        );
        self.agents.lock().insert(
            agent_id.clone(),
            AgentInfo {
                session_id: session_id.clone(),
                workspace_path: workspace_path.clone(),
                shutdown_tx: Some(shutdown_tx),
            },
        );
        AgentHandle::new(agent_id, session_id, workspace_path)
    }

    /// Register a fake agent for testing (bypasses spawn)
    #[cfg(test)]
    fn register_test_agent(&self, agent_id: &AgentId, session_id: &str) {
        self.agents.lock().insert(
            agent_id.clone(),
            AgentInfo {
                session_id: session_id.to_string(),
                workspace_path: PathBuf::new(),
                shutdown_tx: None,
            },
        );
    }
}

#[async_trait]
impl<S: SessionAdapter> AgentAdapter for ClaudeAgentAdapter<S> {
    async fn spawn(
        &self,
        config: AgentSpawnConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        tracing::debug!(
            agent_id = %config.agent_id,
            workspace_path = %config.workspace_path.display(),
            "spawning agent"
        );

        // Precondition: cwd must exist if specified
        if let Some(ref cwd) = config.cwd {
            if !cwd.exists() {
                return Err(AgentAdapterError::SpawnFailed(format!(
                    "working directory does not exist: {}",
                    cwd.display()
                )));
            }
        }

        // 1. Prepare workspace (directory and settings)
        prepare_workspace(&config.workspace_path, &config.project_root)
            .await
            .map_err(|e| AgentAdapterError::WorkspaceError(e.to_string()))?;

        // 2. Determine effective working directory
        let cwd = config
            .cwd
            .clone()
            .unwrap_or_else(|| config.workspace_path.clone());

        // 3. Use configured environment
        let env = config.env.clone();
        let unset_env = config.unset_env.clone();

        // 4. Generate friendly session name (UUID still used for --session-id flag)
        let session_name = generate_session_name(&config.job_name, &config.agent_name);

        // 5. Spawn the underlying session
        let command = augment_command_for_skip_permissions(&config.command);
        let spawned_id = self
            .sessions
            .spawn(&session_name, &cwd, &command, &env, &unset_env)
            .await
            .map_err(|e| AgentAdapterError::SessionError(e.to_string()))?;

        tracing::info!(
            agent_id = %config.agent_id,
            session_id = %spawned_id,
            "agent session spawned"
        );

        // 5a. Handle interactive startup prompts (bypass permissions, workspace trust, login)
        handle_startup_prompts(
            &self.sessions,
            &spawned_id,
            &config.agent_id,
            command.contains("--dangerously-skip-permissions"),
        )
        .await?;

        // Start watcher and register agent
        let process_name = extract_process_name(&config.command);
        let watcher_config = WatcherConfig {
            log_session_id: config.agent_id.to_string(),
            agent_id: config.agent_id,
            tmux_session_id: spawned_id,
            project_path: cwd,
            process_name,
            owner: config.owner,
        };
        Ok(self.start_watcher_and_register(watcher_config, config.workspace_path, event_tx))
    }

    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        tracing::debug!(
            agent_id = %config.agent_id,
            session_id = %config.session_id,
            workspace_path = %config.workspace_path.display(),
            "reconnecting to existing agent session"
        );

        let watcher_config = WatcherConfig {
            log_session_id: config.agent_id.to_string(),
            agent_id: config.agent_id,
            tmux_session_id: config.session_id,
            project_path: config.workspace_path.clone(),
            process_name: config.process_name,
            owner: config.owner,
        };
        Ok(self.start_watcher_and_register(watcher_config, config.workspace_path, event_tx))
    }

    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError> {
        let session_id = {
            let agents = self.agents.lock();
            agents
                .get(agent_id)
                .map(|info| info.session_id.clone())
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?
        };

        // Clear current input: Esc, pause, Esc, pause
        let key_pause = Duration::from_millis(50);
        for _ in 0..2 {
            self.sessions
                .send(&session_id, "Escape")
                .await
                .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))?;
            tokio::time::sleep(key_pause).await;
        }

        // Send literal text
        self.sessions
            .send_literal(&session_id, input)
            .await
            .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))?;

        // Scale delay with input length: TUI re-renders per keystroke
        let text_settle = Duration::from_millis((100 + input.len() as u64).min(2000));
        tokio::time::sleep(text_settle).await;

        self.sessions
            .send_enter(&session_id)
            .await
            .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError> {
        let (session_id, shutdown_tx) = {
            let mut agents = self.agents.lock();
            let info = agents
                .remove(agent_id)
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?;
            (info.session_id, info.shutdown_tx)
        };

        // Stop the watcher first
        if let Some(tx) = shutdown_tx {
            let _ = tx.send(());
        }

        // Kill the session
        self.sessions
            .kill(&session_id)
            .await
            .map_err(|e| AgentAdapterError::KillFailed(e.to_string()))
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError> {
        let (session_id, workspace_path) = {
            let agents = self.agents.lock();
            let info = agents
                .get(agent_id)
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?;
            (info.session_id.clone(), info.workspace_path.clone())
        };

        // Check if session is alive
        let is_alive = self.sessions.is_alive(&session_id).await.unwrap_or(false);

        if !is_alive {
            return Ok(AgentState::SessionGone);
        }

        // Try to parse session log (use agent_id, not session_id, to match --session-id {agent_id})
        let log_session_id = agent_id.to_string();
        if let Some(log_path) = super::watcher::find_session_log(&workspace_path, &log_session_id) {
            return Ok(parse_session_log(&log_path));
        }

        // Fallback: assume working if session is alive
        Ok(AgentState::Working)
    }

    async fn session_log_size(&self, agent_id: &AgentId) -> Option<u64> {
        let workspace_path = {
            let agents = self.agents.lock();
            agents.get(agent_id).map(|info| info.workspace_path.clone())
        }?;

        let log_session_id = agent_id.to_string();
        let log_path = super::watcher::find_session_log(&workspace_path, &log_session_id)?;
        tokio::fs::metadata(&log_path).await.ok().map(|m| m.len())
    }

    async fn last_assistant_message(&self, agent_id: &AgentId) -> Option<String> {
        let workspace_path = {
            let agents = self.agents.lock();
            agents.get(agent_id).map(|info| info.workspace_path.clone())
        }?;

        let log_session_id = agent_id.to_string();
        let log_path = super::watcher::find_session_log(&workspace_path, &log_session_id)?;
        super::watcher::extract_last_assistant_text(&log_path)
    }
}

/// Check for a prompt and log the result.
async fn check_prompt<S: SessionAdapter>(
    sessions: &S,
    session_id: &str,
    agent_id: &AgentId,
    name: &str,
    check: &PromptCheck<'_>,
) -> Result<PromptResult, AgentAdapterError> {
    let r = poll_for_prompt(
        sessions,
        session_id,
        crate::env::prompt_poll_max_attempts(),
        check,
    )
    .await?;
    log_prompt_result(agent_id, name, &r);
    Ok(r)
}

/// Handle interactive startup prompts: bypass permissions, workspace trust, login.
async fn handle_startup_prompts<S: SessionAdapter>(
    sessions: &S,
    session_id: &str,
    agent_id: &AgentId,
    has_skip_permissions: bool,
) -> Result<(), AgentAdapterError> {
    if has_skip_permissions {
        check_prompt(
            sessions,
            session_id,
            agent_id,
            "bypass permissions",
            &PromptCheck {
                detect: &["Bypass Permissions mode", "1. No", "2. Yes"],
                match_any: false,
                response: Some("2"),
                check_errors: true,
            },
        )
        .await?;
    }
    check_prompt(
        sessions,
        session_id,
        agent_id,
        "workspace trust",
        &PromptCheck {
            detect: &["Accessing workspace", "1. Yes", "2. No"],
            match_any: false,
            response: Some("1"),
            check_errors: true,
        },
    )
    .await?;
    let r = check_prompt(
        sessions,
        session_id,
        agent_id,
        "login",
        &PromptCheck {
            detect: &["Select login method", "Choose the text style"],
            match_any: true,
            response: None,
            check_errors: false,
        },
    )
    .await?;
    if r == PromptResult::Handled {
        tracing::error!(%agent_id, "Claude Code is not authenticated — login/onboarding prompt detected");
        let _ = sessions.kill(session_id).await;
        return Err(AgentAdapterError::SpawnFailed(
            "Claude Code is not authenticated. Run `claude` once manually to complete setup."
                .into(),
        ));
    }
    Ok(())
}

/// Prepare workspace for agent execution
async fn prepare_workspace(workspace_path: &Path, project_root: &Path) -> std::io::Result<()> {
    // Ensure workspace exists
    tokio::fs::create_dir_all(workspace_path).await?;

    // Copy settings if they exist
    let project_settings = project_root.join(".claude/settings.json");
    if tokio::fs::try_exists(&project_settings)
        .await
        .unwrap_or(false)
    {
        let claude_dir = workspace_path.join(".claude");
        tokio::fs::create_dir_all(&claude_dir).await?;
        tokio::fs::copy(&project_settings, claude_dir.join("settings.local.json")).await?;
    }

    Ok(())
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
