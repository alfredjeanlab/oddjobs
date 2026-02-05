// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Effect executor

use crate::{RuntimeDeps, Scheduler};
use oj_adapters::{
    AgentAdapter, AgentReconnectConfig, AgentSpawnConfig, NotifyAdapter, SessionAdapter,
};
use oj_core::{Clock, Effect, Event};
use oj_storage::MaterializedState;
use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::mpsc;

/// Errors that can occur during effect execution
#[derive(Debug, Error)]
pub enum ExecuteError {
    #[error("session error: {0}")]
    Session(#[from] oj_adapters::session::SessionError),
    #[error("agent error: {0}")]
    Agent(#[from] oj_adapters::AgentError),
    #[error("storage error: {0}")]
    Storage(#[from] oj_storage::WalError),
    #[error("workspace not found: {0}")]
    WorkspaceNotFound(String),
    #[error("shell execution error: {0}")]
    Shell(String),
}

/// Executes effects using the configured adapters
pub struct Executor<S, A, N, C: Clock> {
    sessions: S,
    agents: A,
    notifier: N,
    state: Arc<Mutex<MaterializedState>>,
    scheduler: Arc<Mutex<Scheduler>>,
    clock: C,
    /// Channel for emitting events from agent watchers
    event_tx: mpsc::Sender<Event>,
}

impl<S, A, N, C> Executor<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Create a new executor
    pub fn new(
        deps: RuntimeDeps<S, A, N>,
        scheduler: Arc<Mutex<Scheduler>>,
        clock: C,
        event_tx: mpsc::Sender<Event>,
    ) -> Self {
        Self {
            sessions: deps.sessions,
            agents: deps.agents,
            notifier: deps.notifier,
            state: deps.state,
            scheduler,
            clock,
            event_tx,
        }
    }

    /// Get a reference to the clock
    pub fn clock(&self) -> &C {
        &self.clock
    }

    /// Execute a single effect with tracing
    ///
    /// Returns an optional event that should be fed back into the event loop.
    pub async fn execute(&self, effect: Effect) -> Result<Option<Event>, ExecuteError> {
        let op_name = effect.name();
        let span = tracing::info_span!("effect", effect = op_name);
        let _guard = span.enter();

        tracing::info!(fields = ?effect.fields(), "executing");

        let start = std::time::Instant::now();
        let result = self.execute_inner(effect).await;
        let elapsed = start.elapsed();

        match &result {
            Ok(event) => tracing::info!(
                elapsed_ms = elapsed.as_millis() as u64,
                has_event = event.is_some(),
                "completed"
            ),
            Err(e) => tracing::error!(
                elapsed_ms = elapsed.as_millis() as u64,
                error = %e,
                "failed"
            ),
        }

        result
    }

    /// Inner execution logic for a single effect
    async fn execute_inner(&self, effect: Effect) -> Result<Option<Event>, ExecuteError> {
        match effect {
            // === Event emission ===
            Effect::Emit { event } => {
                // Apply state change immediately (for effects that depend on it)
                {
                    let mut state = self.state.lock();
                    state.apply_event(&event);
                }
                // Return the event so it can be written to WAL for durability
                Ok(Some(event))
            }

            // === Agent-level effects ===
            Effect::SpawnAgent {
                agent_id,
                agent_name,
                pipeline_id,
                agent_run_id,
                workspace_path,
                input,
                command,
                env,
                cwd,
                session_config,
            } => {
                // Build agent configuration from effect fields
                let config = AgentSpawnConfig {
                    agent_id: agent_id.clone(),
                    agent_name,
                    command,
                    env,
                    workspace_path: workspace_path.clone(),
                    cwd,
                    prompt: input.get("prompt").cloned().unwrap_or_default(),
                    pipeline_name: input
                        .get("name")
                        .cloned()
                        .unwrap_or_else(|| pipeline_id.to_string()),
                    pipeline_id: pipeline_id.to_string(),
                    project_root: workspace_path,
                    session_config,
                };

                // Spawn agent (this starts the watcher that emits events)
                let handle = self.agents.spawn(config, self.event_tx.clone()).await?;

                // Emit SessionCreated so state tracks the session_id
                let event = Event::SessionCreated {
                    id: oj_core::SessionId::new(handle.session_id),
                    pipeline_id,
                    agent_run_id,
                };
                {
                    let mut state = self.state.lock();
                    state.apply_event(&event);
                }
                Ok(Some(event))
            }

            Effect::SendToAgent { agent_id, input } => {
                self.agents.send(&agent_id, &input).await?;
                Ok(None)
            }

            Effect::KillAgent { agent_id } => {
                self.agents.kill(&agent_id).await?;
                Ok(None)
            }

            // === Session-level effects ===
            Effect::SendToSession { session_id, input } => {
                self.sessions.send(session_id.as_str(), &input).await?;
                Ok(None)
            }

            Effect::KillSession { session_id } => {
                self.sessions.kill(session_id.as_str()).await?;
                Ok(None)
            }

            // === Workspace effects ===
            Effect::CreateWorkspace {
                workspace_id,
                path,
                owner,
                workspace_type,
                repo_root,
                branch,
                start_point,
            } => {
                let is_worktree = workspace_type.as_deref() == Some("worktree");

                // Create workspace record first via state event
                let create_event = Event::WorkspaceCreated {
                    id: workspace_id.clone(),
                    path: path.clone(),
                    branch: branch.clone(),
                    owner,
                    workspace_type,
                };
                {
                    let mut state = self.state.lock();
                    state.apply_event(&create_event);
                }

                if is_worktree {
                    // Create parent directory
                    if let Some(parent) = path.parent() {
                        tokio::fs::create_dir_all(parent).await.map_err(|e| {
                            ExecuteError::Shell(format!(
                                "failed to create workspace parent dir: {}",
                                e
                            ))
                        })?;
                    }

                    // Run: git -C <repo_root> worktree add -b <branch> <path> <start_point>
                    let repo_root = repo_root.ok_or_else(|| {
                        ExecuteError::Shell("repo_root required for worktree workspace".to_string())
                    })?;
                    let branch = branch.ok_or_else(|| {
                        ExecuteError::Shell("branch required for worktree workspace".to_string())
                    })?;
                    let start_point = start_point.unwrap_or_else(|| "HEAD".to_string());

                    let path_str = path.display().to_string();
                    let mut cmd = tokio::process::Command::new("git");
                    cmd.args([
                        "-C",
                        &repo_root.display().to_string(),
                        "worktree",
                        "add",
                        "-b",
                        &branch,
                        &path_str,
                        &start_point,
                    ])
                    .env_remove("GIT_DIR")
                    .env_remove("GIT_WORK_TREE");
                    let output = oj_adapters::subprocess::run_with_timeout(
                        cmd,
                        oj_adapters::subprocess::GIT_WORKTREE_TIMEOUT,
                        "git worktree add",
                    )
                    .await
                    .map_err(ExecuteError::Shell)?;

                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let fail_event = Event::WorkspaceFailed {
                            id: workspace_id.clone(),
                            reason: format!("git worktree add failed: {}", stderr.trim()),
                        };
                        {
                            let mut state = self.state.lock();
                            state.apply_event(&fail_event);
                        }
                        return Err(ExecuteError::Shell(format!(
                            "git worktree add failed: {}",
                            stderr.trim()
                        )));
                    }
                } else {
                    // Create empty directory (folder workspace)
                    tokio::fs::create_dir_all(&path).await.map_err(|e| {
                        ExecuteError::Shell(format!("failed to create workspace dir: {}", e))
                    })?;
                }

                // Update status to Ready
                let ready_event = Event::WorkspaceReady {
                    id: workspace_id.clone(),
                };
                {
                    let mut state = self.state.lock();
                    state.apply_event(&ready_event);
                }

                // Return the ready event for WAL persistence
                Ok(Some(ready_event))
            }

            Effect::DeleteWorkspace { workspace_id } => {
                // Look up workspace path and branch
                let (workspace_path, workspace_branch) = {
                    let state = self.state.lock();
                    let ws = state
                        .workspaces
                        .get(workspace_id.as_str())
                        .ok_or_else(|| ExecuteError::WorkspaceNotFound(workspace_id.to_string()))?;
                    (ws.path.clone(), ws.branch.clone())
                };

                // Update status to Cleaning (transient, not persisted)
                {
                    let mut state = self.state.lock();
                    if let Some(workspace) = state.workspaces.get_mut(workspace_id.as_str()) {
                        workspace.status = oj_core::WorkspaceStatus::Cleaning;
                    }
                }

                // If the workspace is a git worktree, unregister it first
                let dot_git = workspace_path.join(".git");
                if tokio::fs::symlink_metadata(&dot_git)
                    .await
                    .map(|m| m.is_file())
                    .unwrap_or(false)
                {
                    // Best-effort: git worktree remove --force
                    // Run from within the worktree so git can locate the parent repo.
                    let mut cmd = tokio::process::Command::new("git");
                    cmd.arg("worktree")
                        .arg("remove")
                        .arg("--force")
                        .arg(&workspace_path)
                        .current_dir(&workspace_path);
                    let _ = oj_adapters::subprocess::run_with_timeout(
                        cmd,
                        oj_adapters::subprocess::GIT_WORKTREE_TIMEOUT,
                        "git worktree remove",
                    )
                    .await;

                    // Best-effort: clean up the branch
                    if let Some(ref branch) = workspace_branch {
                        // Find the repo root from the worktree's .git file
                        if let Ok(contents) = tokio::fs::read_to_string(&dot_git).await {
                            // .git file contains: gitdir: /path/to/repo/.git/worktrees/<name>
                            if let Some(gitdir) = contents.trim().strip_prefix("gitdir: ") {
                                // Navigate up from .git/worktrees/<name> to .git, then parent
                                let gitdir_path = std::path::Path::new(gitdir);
                                if let Some(repo_root) = gitdir_path
                                    .parent()
                                    .and_then(|p| p.parent())
                                    .and_then(|p| p.parent())
                                {
                                    let mut cmd = tokio::process::Command::new("git");
                                    cmd.args([
                                        "-C",
                                        &repo_root.display().to_string(),
                                        "branch",
                                        "-D",
                                        branch,
                                    ])
                                    .env_remove("GIT_DIR")
                                    .env_remove("GIT_WORK_TREE");
                                    let _ = oj_adapters::subprocess::run_with_timeout(
                                        cmd,
                                        oj_adapters::subprocess::GIT_WORKTREE_TIMEOUT,
                                        "git branch delete",
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }

                // Remove workspace directory (in case worktree remove left remnants)
                if workspace_path.exists() {
                    tokio::fs::remove_dir_all(&workspace_path)
                        .await
                        .map_err(|e| {
                            ExecuteError::Shell(format!("failed to remove workspace dir: {}", e))
                        })?;
                }

                // Delete workspace record
                let delete_event = Event::WorkspaceDeleted {
                    id: workspace_id.clone(),
                };
                {
                    let mut state = self.state.lock();
                    state.apply_event(&delete_event);
                }

                // Return the delete event for WAL persistence
                Ok(Some(delete_event))
            }

            // === Timer effects ===
            Effect::SetTimer { id, duration } => {
                let now = oj_core::Clock::now(&self.clock);
                self.scheduler
                    .lock()
                    .set_timer(id.to_string(), duration, now);
                Ok(None)
            }

            Effect::CancelTimer { id } => {
                self.scheduler.lock().cancel_timer(id.as_str());
                Ok(None)
            }

            // === Shell effects ===
            Effect::Shell {
                pipeline_id,
                step,
                command,
                cwd,
                env,
            } => {
                let event_tx = self.event_tx.clone();

                tokio::spawn(async move {
                    tracing::info!(
                        %pipeline_id,
                        step,
                        %command,
                        cwd = %cwd.display(),
                        "running shell command"
                    );

                    let wrapped = format!("set -euo pipefail\n{command}");
                    let result = tokio::process::Command::new("bash")
                        .arg("-c")
                        .arg(&wrapped)
                        .current_dir(&cwd)
                        .envs(&env)
                        .output()
                        .await;

                    let (exit_code, stdout, stderr) = match result {
                        Ok(output) => {
                            let stdout_str = if output.stdout.is_empty() {
                                None
                            } else {
                                let s = String::from_utf8_lossy(&output.stdout).into_owned();
                                tracing::info!(
                                    %pipeline_id,
                                    step,
                                    cwd = %cwd.display(),
                                    stdout = %s,
                                    "shell stdout"
                                );
                                Some(s)
                            };
                            let stderr_str = if output.stderr.is_empty() {
                                None
                            } else {
                                let s = String::from_utf8_lossy(&output.stderr).into_owned();
                                tracing::warn!(
                                    %pipeline_id,
                                    step,
                                    cwd = %cwd.display(),
                                    stderr = %s,
                                    "shell stderr"
                                );
                                Some(s)
                            };
                            (output.status.code().unwrap_or(-1), stdout_str, stderr_str)
                        }
                        Err(e) => {
                            tracing::error!(
                                %pipeline_id,
                                step,
                                cwd = %cwd.display(),
                                error = %e,
                                "shell execution failed"
                            );
                            (-1, None, None)
                        }
                    };

                    let event = Event::ShellExited {
                        pipeline_id,
                        step,
                        exit_code,
                        stdout,
                        stderr,
                    };

                    if let Err(e) = event_tx.send(event).await {
                        tracing::error!("failed to send ShellExited: {}", e);
                    }
                });

                Ok(None)
            }

            // === Worker effects ===
            Effect::PollQueue {
                worker_name,
                list_command,
                cwd,
            } => {
                let event_tx = self.event_tx.clone();

                tokio::spawn(async move {
                    tracing::info!(
                        %worker_name,
                        %list_command,
                        cwd = %cwd.display(),
                        "polling queue"
                    );

                    let wrapped = format!("set -euo pipefail\n{list_command}");
                    let result = tokio::process::Command::new("bash")
                        .arg("-c")
                        .arg(&wrapped)
                        .current_dir(&cwd)
                        .output()
                        .await;

                    let items = match result {
                        Ok(output) if output.status.success() => {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            match serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                                Ok(items) => items,
                                Err(e) => {
                                    tracing::warn!(
                                        %worker_name,
                                        error = %e,
                                        stdout = %stdout,
                                        "failed to parse queue list output as JSON array"
                                    );
                                    vec![]
                                }
                            }
                        }
                        Ok(output) => {
                            if !output.stderr.is_empty() {
                                tracing::warn!(
                                    %worker_name,
                                    stderr = %String::from_utf8_lossy(&output.stderr),
                                    "queue list command failed"
                                );
                            }
                            vec![]
                        }
                        Err(e) => {
                            tracing::error!(
                                %worker_name,
                                error = %e,
                                "queue list command execution failed"
                            );
                            vec![]
                        }
                    };

                    let event = Event::WorkerPollComplete { worker_name, items };

                    if let Err(e) = event_tx.send(event).await {
                        tracing::error!("failed to send WorkerPollComplete: {}", e);
                    }
                });

                Ok(None)
            }

            Effect::TakeQueueItem {
                worker_name,
                take_command,
                cwd,
                item_id,
                item,
            } => {
                let event_tx = self.event_tx.clone();

                tokio::spawn(async move {
                    tracing::info!(
                        %worker_name,
                        %take_command,
                        cwd = %cwd.display(),
                        "taking queue item"
                    );

                    let wrapped = format!("set -euo pipefail\n{take_command}");
                    let result = tokio::process::Command::new("bash")
                        .arg("-c")
                        .arg(&wrapped)
                        .current_dir(&cwd)
                        .output()
                        .await;

                    let (exit_code, stderr) = match result {
                        Ok(output) => {
                            if output.status.success() && !output.stdout.is_empty() {
                                tracing::info!(
                                    %worker_name,
                                    stdout = %String::from_utf8_lossy(&output.stdout),
                                    "take command stdout"
                                );
                            }
                            let stderr_str = if output.stderr.is_empty() {
                                None
                            } else {
                                let s = String::from_utf8_lossy(&output.stderr).into_owned();
                                if !output.status.success() {
                                    tracing::warn!(
                                        %worker_name,
                                        exit_code = output.status.code().unwrap_or(-1),
                                        stderr = %s,
                                        "take command failed"
                                    );
                                }
                                Some(s)
                            };
                            (output.status.code().unwrap_or(-1), stderr_str)
                        }
                        Err(e) => {
                            tracing::error!(
                                %worker_name,
                                error = %e,
                                "take command execution failed"
                            );
                            (-1, None)
                        }
                    };

                    let event = Event::WorkerTakeComplete {
                        worker_name,
                        item_id,
                        item,
                        exit_code,
                        stderr,
                    };

                    if let Err(e) = event_tx.send(event).await {
                        tracing::error!("failed to send WorkerTakeComplete: {}", e);
                    }
                });

                Ok(None)
            }

            // === Notification effects ===
            Effect::Notify { title, message } => {
                if let Err(e) = self.notifier.notify(&title, &message).await {
                    tracing::warn!(%title, error = %e, "notification send failed");
                }
                Ok(None)
            }
        }
    }

    /// Kill a tmux session by ID.
    pub async fn kill_session(&self, session_id: &str) -> Result<(), ExecuteError> {
        self.sessions.kill(session_id).await?;
        Ok(())
    }

    /// Reconnect monitoring for an already-running agent session.
    ///
    /// Calls the adapter's `reconnect` method to re-establish background
    /// monitoring without spawning a new session.
    pub async fn reconnect_agent(&self, config: AgentReconnectConfig) -> Result<(), ExecuteError> {
        self.agents.reconnect(config, self.event_tx.clone()).await?;
        Ok(())
    }

    /// Execute multiple effects in order
    ///
    /// Returns any events that were produced by effects (to be fed back into the event loop).
    pub async fn execute_all(&self, effects: Vec<Effect>) -> Result<Vec<Event>, ExecuteError> {
        let mut result_events = Vec::new();
        for effect in effects {
            if let Some(event) = self.execute(effect).await? {
                result_events.push(event);
            }
        }
        Ok(result_events)
    }

    /// Get a reference to the state
    pub fn state(&self) -> Arc<Mutex<MaterializedState>> {
        Arc::clone(&self.state)
    }

    /// Get a reference to the scheduler
    pub fn scheduler(&self) -> Arc<Mutex<Scheduler>> {
        Arc::clone(&self.scheduler)
    }

    /// Check if a tmux session is still alive
    pub async fn check_session_alive(&self, session_id: &str) -> bool {
        self.sessions.is_alive(session_id).await.unwrap_or(false)
    }

    /// Get the current state of an agent
    pub async fn get_agent_state(
        &self,
        agent_id: &oj_core::AgentId,
    ) -> Result<oj_core::AgentState, ExecuteError> {
        self.agents
            .get_state(agent_id)
            .await
            .map_err(ExecuteError::Agent)
    }

    /// Get the current size of an agent's session log file in bytes.
    pub async fn get_session_log_size(&self, agent_id: &oj_core::AgentId) -> Option<u64> {
        self.agents.session_log_size(agent_id).await
    }
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
