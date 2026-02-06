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

/// Generate a friendly tmux session name from job context.
///
/// Format: `{job}-{step}-{random}` (oj- prefix added by TmuxAdapter)
///
/// Sanitizes names for tmux compatibility:
/// - Replaces invalid characters with hyphens
/// - Truncates to reasonable length
/// - Adds 4-char random suffix for uniqueness
fn generate_session_name(job_name: &str, step_name: &str) -> String {
    let sanitized_job = sanitize_for_tmux(job_name, 20);
    let sanitized_step = sanitize_for_tmux(step_name, 15);
    let random_suffix = generate_short_random(4);

    format!("{}-{}-{}", sanitized_job, sanitized_step, random_suffix)
}

/// Sanitize a string for use in tmux session names.
///
/// tmux session names cannot contain: colon `:`, period `.`
/// Also replaces other problematic characters for shell friendliness.
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

/// Augment a claude command to add `--allow-dangerously-skip-permissions` if
/// `--dangerously-skip-permissions` is present but the allow flag is not.
fn augment_command_for_skip_permissions(command: &str) -> String {
    if command.contains("--dangerously-skip-permissions")
        && !command.contains("--allow-dangerously-skip-permissions")
    {
        format!("{} --allow-dangerously-skip-permissions", command)
    } else {
        command.to_string()
    }
}

/// Result of checking for the bypass permissions prompt
#[derive(Debug, PartialEq)]
enum BypassPromptResult {
    /// Prompt detected and accepted
    Accepted,
    /// No prompt detected (agent started normally)
    NotPresent,
    /// Unexpected content in the pane
    Unexpected(String),
}

/// Compute the number of 200ms poll attempts for prompt detection.
///
/// Override with `OJ_PROMPT_POLL_MS` env var (e.g. `200` for a single check).
/// Default: 3000ms → 15 attempts.
fn prompt_poll_max_attempts() -> usize {
    crate::env::prompt_poll_max_attempts()
}

/// Check for and auto-accept the bypass permissions confirmation prompt.
///
/// Claude Code with `--dangerously-skip-permissions` shows an interactive dialog:
/// ```text
/// WARNING: Claude Code running in Bypass Permissions mode
/// ...
/// ❯ 1. No, exit
///   2. Yes, I accept
/// ```
///
/// This function detects this prompt and sends "2" to accept it.
async fn handle_bypass_permissions_prompt<S: SessionAdapter>(
    sessions: &S,
    session_id: &str,
    max_attempts: usize,
) -> Result<BypassPromptResult, AgentAdapterError> {
    // Poll for the prompt with a timeout
    // The prompt should appear within a second or two of spawn
    let check_interval = Duration::from_millis(200);

    for attempt in 0..max_attempts {
        // Small delay before first check to let the TUI render
        if attempt > 0 {
            tokio::time::sleep(check_interval).await;
        }

        // Capture the pane output
        let output = match sessions.capture_output(session_id, 50).await {
            Ok(out) => out,
            Err(_) => continue, // Session might not be ready yet
        };

        // Check if we see the bypass permissions prompt
        let has_bypass_warning = output.contains("Bypass Permissions mode");
        let has_no_option = output.contains("1. No");
        let has_yes_option = output.contains("2. Yes");

        if has_bypass_warning && has_no_option && has_yes_option {
            tracing::info!(
                session_id,
                "detected bypass permissions prompt, sending '2' to accept"
            );

            // Send "2" to select "Yes, I accept"
            sessions
                .send(session_id, "2")
                .await
                .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))?;

            return Ok(BypassPromptResult::Accepted);
        }

        // If we see an error, report it
        if output.contains("Error:") || output.contains("error:") {
            return Ok(BypassPromptResult::Unexpected(output));
        }
    }

    // Timeout - no prompt detected, assume agent started normally
    Ok(BypassPromptResult::NotPresent)
}

/// Result of checking for the workspace trust prompt
#[derive(Debug, PartialEq)]
enum WorkspaceTrustResult {
    /// Prompt detected and accepted
    Accepted,
    /// No prompt detected (agent started normally)
    NotPresent,
    /// Unexpected content in the pane
    Unexpected(String),
}

/// Check for and auto-accept the workspace trust prompt.
///
/// Claude Code shows an interactive dialog when accessing a workspace:
/// ```text
/// Accessing workspace:
/// /path/to/project
/// ...
/// ❯ 1. Yes, I trust this folder
///   2. No, exit
/// ```
///
/// This function detects this prompt and sends "1" to trust the folder.
async fn handle_workspace_trust_prompt<S: SessionAdapter>(
    sessions: &S,
    session_id: &str,
    max_attempts: usize,
) -> Result<WorkspaceTrustResult, AgentAdapterError> {
    // Poll for the prompt with a timeout
    // The prompt should appear within a second or two of spawn
    let check_interval = Duration::from_millis(200);

    for attempt in 0..max_attempts {
        // Small delay before first check to let the TUI render
        if attempt > 0 {
            tokio::time::sleep(check_interval).await;
        }

        // Capture the pane output
        let output = match sessions.capture_output(session_id, 50).await {
            Ok(out) => out,
            Err(_) => continue, // Session might not be ready yet
        };

        // Check if we see the workspace trust prompt
        let has_workspace_msg = output.contains("Accessing workspace");
        let has_yes_option = output.contains("1. Yes");
        let has_no_option = output.contains("2. No");

        if has_workspace_msg && has_yes_option && has_no_option {
            tracing::info!(
                session_id,
                "detected workspace trust prompt, sending '1' to trust"
            );

            // Send "1" to select "Yes, I trust this folder"
            sessions
                .send(session_id, "1")
                .await
                .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))?;

            return Ok(WorkspaceTrustResult::Accepted);
        }

        // If we see an error, report it
        if output.contains("Error:") || output.contains("error:") {
            return Ok(WorkspaceTrustResult::Unexpected(output));
        }
    }

    // Timeout - no prompt detected, assume agent started normally
    Ok(WorkspaceTrustResult::NotPresent)
}

/// Result of checking for the login/onboarding prompt
#[derive(Debug, PartialEq)]
enum LoginPromptResult {
    /// Login prompt detected — agent is not authenticated
    Detected,
    /// No login prompt detected (agent started normally)
    NotPresent,
}

/// Check for the Claude Code login/onboarding prompt.
///
/// When Claude Code is not authenticated, it shows an interactive dialog asking
/// the user to select a login method or choose text style. If detected, the
/// agent cannot proceed and should fail with a clear error message.
async fn handle_login_prompt<S: SessionAdapter>(
    sessions: &S,
    session_id: &str,
    max_attempts: usize,
) -> Result<LoginPromptResult, AgentAdapterError> {
    let check_interval = Duration::from_millis(200);

    for attempt in 0..max_attempts {
        if attempt > 0 {
            tokio::time::sleep(check_interval).await;
        }

        let output = match sessions.capture_output(session_id, 50).await {
            Ok(out) => out,
            Err(_) => continue,
        };

        // Claude Code login flow shows "Select login method" or onboarding
        // prompts like "Choose the text style"
        if output.contains("Select login method") || output.contains("Choose the text style") {
            return Ok(LoginPromptResult::Detected);
        }
    }

    Ok(LoginPromptResult::NotPresent)
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

        // 4. Generate friendly session name (UUID still used for --session-id flag)
        let session_name = generate_session_name(&config.job_name, &config.agent_name);

        // 5. Spawn the underlying session
        let command = augment_command_for_skip_permissions(&config.command);
        let spawned_id = self
            .sessions
            .spawn(&session_name, &cwd, &command, &env)
            .await
            .map_err(|e| AgentAdapterError::SessionError(e.to_string()))?;

        tracing::info!(
            agent_id = %config.agent_id,
            session_id = %spawned_id,
            "agent session spawned"
        );

        // 5a. Apply session configuration (status bar, colors, title)
        if let Some(tmux_config) = config.session_config.get("tmux") {
            if let Err(e) = self.sessions.configure(&spawned_id, tmux_config).await {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    error = %e,
                    "failed to configure session (non-fatal)"
                );
            }
        }

        // 5b. Handle bypass permissions prompt if present
        if command.contains("--dangerously-skip-permissions") {
            match handle_bypass_permissions_prompt(
                &self.sessions,
                &spawned_id,
                prompt_poll_max_attempts(),
            )
            .await?
            {
                BypassPromptResult::Accepted => {
                    tracing::info!(agent_id = %config.agent_id, "bypass permissions prompt accepted");
                }
                BypassPromptResult::NotPresent => {
                    tracing::debug!(agent_id = %config.agent_id, "no bypass permissions prompt detected");
                }
                BypassPromptResult::Unexpected(output) => {
                    tracing::warn!(
                        agent_id = %config.agent_id,
                        output = %output,
                        "unexpected output while checking for bypass permissions prompt"
                    );
                    // Continue anyway - the watcher will detect if something is wrong
                }
            }
        }

        // 5c. Handle workspace trust prompt if present
        match handle_workspace_trust_prompt(&self.sessions, &spawned_id, prompt_poll_max_attempts())
            .await?
        {
            WorkspaceTrustResult::Accepted => {
                tracing::info!(agent_id = %config.agent_id, "workspace trust prompt accepted");
            }
            WorkspaceTrustResult::NotPresent => {
                tracing::debug!(agent_id = %config.agent_id, "no workspace trust prompt detected");
            }
            WorkspaceTrustResult::Unexpected(output) => {
                tracing::warn!(
                    agent_id = %config.agent_id,
                    output = %output,
                    "unexpected output while checking for workspace trust prompt"
                );
                // Continue anyway - the watcher will detect if something is wrong
            }
        }

        // 5d. Check for login/onboarding prompt (agent not authenticated)
        if let LoginPromptResult::Detected =
            handle_login_prompt(&self.sessions, &spawned_id, prompt_poll_max_attempts()).await?
        {
            tracing::error!(
                agent_id = %config.agent_id,
                "Claude Code is not authenticated — login/onboarding prompt detected"
            );
            // Kill the session since the agent can't proceed
            let _ = self.sessions.kill(&spawned_id).await;
            return Err(AgentAdapterError::SpawnFailed(
                "Claude Code is not authenticated. Run `claude` once manually to complete setup."
                    .to_string(),
            ));
        }

        // 6. Start background watcher (with optional log entry extraction)
        // Pass agent_id (UUID) for log lookup (matches --session-id {agent_id} passed to claude)
        // Pass spawned_id (friendly tmux name with oj- prefix) for liveness checking
        let process_name = extract_process_name(&config.command);
        let watcher_config = WatcherConfig {
            agent_id: config.agent_id.clone(),
            log_session_id: config.agent_id.to_string(),
            tmux_session_id: spawned_id.clone(),
            project_path: cwd.clone(),
            process_name,
        };
        let shutdown_tx = start_watcher(
            watcher_config,
            self.sessions.clone(),
            event_tx,
            self.log_entry_tx.clone(),
        );

        // 7. Store agent info (use spawned_id for session operations like kill/send)
        {
            let mut agents = self.agents.lock();
            agents.insert(
                config.agent_id.clone(),
                AgentInfo {
                    session_id: spawned_id.clone(),
                    workspace_path: config.workspace_path.clone(),
                    shutdown_tx: Some(shutdown_tx),
                },
            );
        }

        Ok(AgentHandle::new(
            config.agent_id,
            spawned_id,
            config.workspace_path,
        ))
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

        // Start background watcher (same as spawn step 6)
        // Use agent_id for log session ID (matches --session-id {agent_id})
        // config.session_id is the tmux session ID (has oj- prefix)
        let watcher_config = WatcherConfig {
            agent_id: config.agent_id.clone(),
            log_session_id: config.agent_id.to_string(),
            tmux_session_id: config.session_id.clone(),
            project_path: config.workspace_path.clone(),
            process_name: config.process_name.clone(),
        };
        let shutdown_tx = start_watcher(
            watcher_config,
            self.sessions.clone(),
            event_tx,
            self.log_entry_tx.clone(),
        );

        // Store agent info (same as spawn step 7)
        let session_id = config.session_id.clone();
        {
            let mut agents = self.agents.lock();
            agents.insert(
                config.agent_id.clone(),
                AgentInfo {
                    session_id: config.session_id,
                    workspace_path: config.workspace_path.clone(),
                    shutdown_tx: Some(shutdown_tx),
                },
            );
        }

        Ok(AgentHandle::new(
            config.agent_id,
            session_id,
            config.workspace_path,
        ))
    }

    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError> {
        let session_id = {
            let agents = self.agents.lock();
            agents
                .get(agent_id)
                .map(|info| info.session_id.clone())
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?
        };

        let key_pause = Duration::from_millis(50);

        // Clear current input: Esc, pause, Esc
        self.sessions
            .send(&session_id, "Escape")
            .await
            .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))?;

        tokio::time::sleep(key_pause).await;

        self.sessions
            .send(&session_id, "Escape")
            .await
            .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))?;

        tokio::time::sleep(key_pause).await;

        // Send literal text
        self.sessions
            .send_literal(&session_id, input)
            .await
            .map_err(|e| AgentAdapterError::SendFailed(e.to_string()))?;

        // Wait for the TUI to process all characters before pressing Enter.
        // Scale the delay with input length: the TUI re-renders per keystroke,
        // so longer messages need more time. Base 100ms, +1ms per char, cap 2s.
        let text_settle = Duration::from_millis((100 + input.len() as u64).min(2000));
        tokio::time::sleep(text_settle).await;

        // Send Enter to submit
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
