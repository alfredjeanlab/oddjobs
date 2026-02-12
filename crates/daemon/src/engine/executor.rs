// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Effect executor

use crate::adapters::subprocess::{run_with_timeout, QUEUE_COMMAND_TIMEOUT, SHELL_COMMAND_TIMEOUT};
use crate::adapters::{
    AgentAdapter, AgentConfig, AgentReconnectConfig, NotifyAdapter, WorkspaceAdapter,
};
use crate::engine::{scheduler::Scheduler, RuntimeDeps};
use crate::storage::MaterializedState;
use oj_core::{Clock, Effect, Event};
use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::mpsc;

/// Errors that can occur during effect execution
#[derive(Debug, Error)]
pub enum ExecuteError {
    #[error("agent error: {0}")]
    Agent(#[from] crate::adapters::AgentAdapterError),
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::WalError),
    #[error("workspace not found: {0}")]
    WorkspaceNotFound(String),
    #[error("shell execution error: {0}")]
    Shell(String),
}

/// Executes effects using the configured adapters
pub struct Executor<A, N, C: Clock> {
    pub(crate) agents: A,
    notifier: N,
    state: Arc<Mutex<MaterializedState>>,
    scheduler: Arc<Mutex<Scheduler>>,
    clock: C,
    /// Channel for emitting events from agent watchers
    event_tx: mpsc::Sender<Event>,
    /// Workspace filesystem adapter (local or noop for k8s).
    workspace: Arc<dyn WorkspaceAdapter>,
}

impl<A, N, C> Executor<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Create a new executor
    pub fn new(
        deps: RuntimeDeps<A, N>,
        scheduler: Arc<Mutex<Scheduler>>,
        clock: C,
        event_tx: mpsc::Sender<Event>,
    ) -> Self {
        Self {
            agents: deps.agents,
            notifier: deps.notifier,
            workspace: deps.workspace,
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

    /// Get a reference to the agent adapter.
    pub fn agents(&self) -> &A {
        &self.agents
    }

    /// Execute a single effect with tracing
    ///
    /// Returns an optional event that should be fed back into the event loop.
    pub async fn execute(&self, effect: Effect) -> Result<Option<Event>, ExecuteError> {
        let info: String =
            effect.fields().iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ");
        let op = effect.name();
        let verbose = effect.verbose();
        if verbose {
            tracing::info!("executing effect={} {}", op, info);
        }

        let start = std::time::Instant::now();
        let result = self.execute_inner(effect).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;
        match &result {
            Ok(ev) if verbose => tracing::info!(event = ev.is_some(), elapsed_ms, "completed"),
            Ok(ev) => tracing::info!(event = ev.is_some(), elapsed_ms, "effect={} {}", op, info),
            Err(e) => tracing::error!(error = %e, elapsed_ms, "error effect={} {}", op, info),
        }
        result
    }

    /// Inner execution logic — dispatches each effect to its handler method.
    async fn execute_inner(&self, effect: Effect) -> Result<Option<Event>, ExecuteError> {
        match effect {
            Effect::Emit { event } => {
                self.state.lock().apply_event(&event);
                Ok(Some(event))
            }
            Effect::SpawnAgent {
                agent_id,
                agent_name,
                owner,
                workspace_path,
                input,
                command,
                env,
                unset_env,
                cwd,
                resume,
                container,
            } => {
                let job_id_str = match &owner {
                    oj_core::OwnerId::Job(id) => id.to_string(),
                    oj_core::OwnerId::Crew(_) => String::new(),
                };

                let mut config =
                    AgentConfig::new(agent_id.clone(), command, workspace_path, owner.clone())
                        .agent_name(agent_name)
                        .env(env)
                        .unset_env(unset_env)
                        .prompt(input.get("prompt").cloned().unwrap_or_default())
                        .job_name(input.get("name").cloned().unwrap_or_else(|| job_id_str.clone()))
                        .job_id(job_id_str);
                config.resume = resume;
                config.container = container;
                if let Some(url) = input.get("source.repo") {
                    config.repo = Some(url.clone());
                }
                // Use source.ref (the start point, e.g. "main") not source.branch
                // (the workspace branch name, e.g. "ws-abc123") — source.branch is
                // a local-only name that doesn't exist on the remote.
                if let Some(r) = input.get("source.ref") {
                    config.branch = Some(r.clone());
                }
                if let Some(cwd) = cwd {
                    config = config.cwd(cwd);
                }

                let agents = self.agents.clone();
                let event_tx = self.event_tx.clone();
                tokio::spawn(async move {
                    match agents.spawn(config, event_tx.clone()).await {
                        Ok(handle) => {
                            let event = Event::AgentSpawned {
                                id: handle.agent_id,
                                owner,
                                runtime: handle.runtime,
                                auth_token: handle.auth_token,
                            };
                            if let Err(e) = event_tx.send(event).await {
                                tracing::error!("failed to send AgentSpawned: {}", e);
                            }
                        }
                        Err(e) => {
                            let event = Event::AgentSpawnFailed {
                                id: agent_id,
                                owner,
                                reason: e.to_string(),
                            };
                            if let Err(e) = event_tx.send(event).await {
                                tracing::error!("failed to send AgentSpawnFailed: {}", e);
                            }
                        }
                    }
                });
                Ok(None)
            }
            Effect::SendToAgent { agent_id, input } => {
                let agents = self.agents.clone();
                tokio::spawn(async move {
                    if let Err(e) = agents.send(&agent_id, &input).await {
                        tracing::warn!(%agent_id, error = %e, "SendToAgent failed");
                    }
                });
                Ok(None)
            }
            Effect::RespondToAgent { agent_id, response } => {
                let agents = self.agents.clone();
                tokio::spawn(async move {
                    if let Err(e) = agents.respond(&agent_id, &response).await {
                        tracing::warn!(%agent_id, error = %e, "RespondToAgent failed");
                    }
                });
                Ok(None)
            }
            Effect::KillAgent { agent_id } => {
                let agents = self.agents.clone();
                tokio::spawn(async move {
                    if let Err(e) = agents.kill(&agent_id).await {
                        tracing::warn!(%agent_id, error = %e, "KillAgent failed");
                    }
                });
                Ok(None)
            }
            Effect::CreateWorkspace {
                workspace_id,
                path,
                owner,
                workspace_type,
                repo_root,
                branch,
                start_point,
            } => {
                crate::engine::workspace::create(
                    &self.state,
                    &self.event_tx,
                    &self.workspace,
                    workspace_id,
                    path,
                    owner,
                    workspace_type,
                    repo_root,
                    branch,
                    start_point,
                )
                .await
            }
            Effect::DeleteWorkspace { workspace_id } => {
                crate::engine::workspace::delete(
                    &self.state,
                    &self.event_tx,
                    &self.workspace,
                    workspace_id,
                )
                .await
            }
            Effect::SetTimer { id, duration } => {
                let now = oj_core::Clock::now(&self.clock);
                self.scheduler.lock().set_timer(id.to_string(), duration, now);
                Ok(None)
            }
            Effect::CancelTimer { id } => {
                self.scheduler.lock().cancel_timer(id.as_str());
                Ok(None)
            }
            Effect::Shell { owner, step, command, cwd, env, container: _container } => {
                self.execute_shell(owner, step, command, cwd, env);
                Ok(None)
            }
            Effect::PollQueue { worker_name, project, list_command, cwd } => {
                self.execute_poll_queue(worker_name, project, list_command, cwd);
                Ok(None)
            }
            Effect::TakeQueueItem { worker_name, project, take_command, cwd, item_id, item } => {
                self.execute_take_queue_item(
                    worker_name,
                    project,
                    take_command,
                    cwd,
                    item_id,
                    item,
                );
                Ok(None)
            }
            Effect::Notify { title, message } => {
                if let Err(e) = self.notifier.notify(&title, &message).await {
                    tracing::warn!(%title, error = %e, "notification send failed");
                }
                Ok(None)
            }
        }
    }

    fn execute_shell(
        &self,
        owner: Option<oj_core::OwnerId>,
        step: String,
        command: String,
        cwd: std::path::PathBuf,
        env: std::collections::HashMap<String, String>,
    ) {
        let event_tx = self.event_tx.clone();
        let job_id = match &owner {
            Some(oj_core::OwnerId::Job(id)) => id.clone(),
            _ => oj_core::JobId::new(""),
        };

        tokio::spawn(async move {
            let owner_str =
                owner.as_ref().map(|o| o.to_string()).unwrap_or_else(|| "none".to_string());
            tracing::info!(
                owner = %owner_str,
                step,
                %command,
                cwd = %cwd.display(),
                "running shell command"
            );

            let wrapped = format!("set -euo pipefail\n{command}");
            let mut cmd = tokio::process::Command::new("bash");
            cmd.arg("-c").arg(&wrapped).current_dir(&cwd).envs(&env);
            let result = run_with_timeout(cmd, SHELL_COMMAND_TIMEOUT, "shell command").await;

            let (exit_code, stdout, stderr) = match result {
                Ok(output) => {
                    let stdout_str = if output.stdout.is_empty() {
                        None
                    } else {
                        let s = String::from_utf8_lossy(&output.stdout).into_owned();
                        tracing::info!(
                            owner = %owner_str,
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
                            owner = %owner_str,
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
                        owner = %owner_str,
                        step,
                        cwd = %cwd.display(),
                        error = %e,
                        "shell execution failed"
                    );
                    (-1, None, None)
                }
            };

            let event = Event::ShellExited { job_id, step, exit_code, stdout, stderr };
            if let Err(e) = event_tx.send(event).await {
                tracing::error!("failed to send ShellExited: {}", e);
            }
        });
    }

    fn execute_poll_queue(
        &self,
        worker: String,
        project: String,
        list_command: String,
        cwd: std::path::PathBuf,
    ) {
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            tracing::info!(%worker, %list_command, cwd = %cwd.display(), "polling queue");

            let wrapped = format!("set -euo pipefail\n{list_command}");
            let mut cmd = tokio::process::Command::new("bash");
            cmd.arg("-c").arg(&wrapped).current_dir(&cwd);
            let result = run_with_timeout(cmd, QUEUE_COMMAND_TIMEOUT, "queue list").await;

            let items = match result {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    match serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                        Ok(items) => items,
                        Err(e) => {
                            tracing::warn!(
                                %worker,
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
                            %worker,
                            stderr = %String::from_utf8_lossy(&output.stderr),
                            "queue list command failed"
                        );
                    }
                    vec![]
                }
                Err(e) => {
                    tracing::error!(%worker, error = %e, "queue list command execution failed");
                    vec![]
                }
            };

            let event = Event::WorkerPolled { worker, project, items };
            if let Err(e) = event_tx.send(event).await {
                tracing::error!("failed to send WorkerPolled: {}", e);
            }
        });
    }

    fn execute_take_queue_item(
        &self,
        worker: String,
        project: String,
        take_command: String,
        cwd: std::path::PathBuf,
        item_id: String,
        item: serde_json::Value,
    ) {
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            tracing::info!(%worker, %take_command, cwd = %cwd.display(), "taking queue item");

            let wrapped = format!("set -euo pipefail\n{take_command}");
            let mut cmd = tokio::process::Command::new("bash");
            cmd.arg("-c").arg(&wrapped).current_dir(&cwd);
            let result = run_with_timeout(cmd, QUEUE_COMMAND_TIMEOUT, "queue take").await;

            let (exit_code, stderr) = match result {
                Ok(output) => {
                    if output.status.success() && !output.stdout.is_empty() {
                        tracing::info!(
                            %worker,
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
                                %worker,
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
                        %worker,
                        error = %e,
                        "take command execution failed"
                    );
                    (-1, None)
                }
            };

            let event = Event::WorkerTook { worker, project, item_id, item, exit_code, stderr };
            if let Err(e) = event_tx.send(event).await {
                tracing::error!("failed to send WorkerTook: {}", e);
            }
        });
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

    /// Get the current state of an agent
    pub async fn get_agent_state(
        &self,
        agent_id: &oj_core::AgentId,
    ) -> Result<oj_core::AgentState, ExecuteError> {
        self.agents.get_state(agent_id).await.map_err(ExecuteError::Agent)
    }

    /// Capture recent terminal output from an agent.
    pub async fn capture_agent_output(
        &self,
        agent_id: &oj_core::AgentId,
        lines: u32,
    ) -> Result<String, ExecuteError> {
        self.agents.capture_output(agent_id, lines).await.map_err(ExecuteError::Agent)
    }

    /// Fetch the full session transcript from an agent's coop sidecar.
    pub async fn fetch_transcript(
        &self,
        agent_id: &oj_core::AgentId,
    ) -> Result<String, ExecuteError> {
        self.agents.fetch_transcript(agent_id).await.map_err(ExecuteError::Agent)
    }
}

#[cfg(test)]
#[path = "executor_tests/mod.rs"]
mod tests;
