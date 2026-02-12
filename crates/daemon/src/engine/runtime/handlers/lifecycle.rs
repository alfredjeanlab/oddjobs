// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job lifecycle event handling (resume, cancel, workspace, shell)

use super::super::Runtime;
use crate::adapters::{AgentAdapter, NotifyAdapter};
use crate::engine::error::RuntimeError;
use oj_core::{
    AgentId, Clock, CrewStatus, Effect, Event, JobId, OwnerId, StepOutcome, StepStatus, TimerId,
    WorkspaceId,
};
use std::collections::HashMap;

impl<A, N, C> Runtime<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_job_resume(
        &self,
        job_id: &JobId,
        message: Option<&str>,
        vars: &HashMap<String, String>,
        kill: bool,
    ) -> Result<Vec<Event>, RuntimeError> {
        let job = self.require_job(job_id.as_str())?;

        let is_failed = job.step == "failed";
        let is_suspended = job.step == "suspended";

        // If job is in terminal "failed" or "suspended" state, find the last failed step
        // from history so we can reset the job to that step for retry.
        let resume_step = if is_failed || is_suspended {
            job.step_history
                .iter()
                .rev()
                .find(|r| matches!(r.outcome, StepOutcome::Failed(_)))
                .map(|r| r.name.clone())
                .ok_or_else(|| {
                    RuntimeError::InvalidRequest("no failed step found in history".into())
                })?
        } else {
            job.step.clone()
        };

        // Determine step type from runbook — do this BEFORE any state mutation
        // so validation failures don't leave half-applied state.
        let runbook = self.cached_runbook(&job.runbook_hash)?;
        let job_def = runbook
            .get_job(&job.kind)
            .ok_or_else(|| RuntimeError::JobDefNotFound(job.kind.clone()))?;
        let step_def = job_def
            .get_step(&resume_step)
            .ok_or_else(|| RuntimeError::StepNotFound(resume_step.clone()))?;

        // Resolve message for agent steps BEFORE emitting any events.
        let resolved_message = if step_def.is_agent() {
            Some(message.unwrap_or("Please continue with the task.").to_string())
        } else {
            None
        };

        // All validation passed — now safe to mutate state.
        let mut result_events = Vec::new();

        // If resuming from "failed" or "suspended", reset the job to the target step
        if is_failed || is_suspended {
            tracing::info!(
                job_id = %job.id,
                failed_step = %resume_step,
                from = if is_suspended { "suspended" } else { "failed" },
                "resuming from terminal state: resetting to step"
            );

            let events = self
                .executor
                .execute(Effect::Emit {
                    event: Event::JobAdvanced { id: job_id.clone(), step: resume_step.clone() },
                })
                .await?;
            result_events.extend(events);
        }

        // Persist var updates if any
        if !vars.is_empty() {
            self.executor
                .execute(Effect::Emit {
                    event: Event::JobUpdated { id: JobId::new(&job.id), vars: vars.clone() },
                })
                .await?;
        }

        // Merge vars for this resume operation
        let merged_inputs: HashMap<String, String> =
            job.vars.iter().map(|(k, v)| (k.clone(), v.clone())).chain(vars.clone()).collect();

        if let Some(msg) = resolved_message {
            let agent_name = step_def
                .agent_name()
                .ok_or_else(|| RuntimeError::AgentNotFound("no agent name in step".into()))?;

            let events = self
                .handle_agent_resume(&job, &resume_step, agent_name, &msg, &merged_inputs, kill)
                .await?;
            result_events.extend(events);
        } else if step_def.is_shell() {
            // Shell step: re-run command
            if message.is_some() {
                tracing::warn!(job_id = %job.id, "resume --message ignored for shell steps");
            }

            let command = step_def
                .shell_command()
                .ok_or_else(|| RuntimeError::InvalidRequest("no shell command in step".into()))?;

            let events = self.handle_shell_resume(&job, &resume_step, command).await?;
            result_events.extend(events);
        } else {
            return Err(RuntimeError::InvalidRequest(format!(
                "resume not supported for step type in step: {}",
                resume_step
            )));
        }

        Ok(result_events)
    }

    pub(crate) async fn handle_job_suspend(
        &self,
        job_id: &JobId,
    ) -> Result<Vec<Event>, RuntimeError> {
        let job = self
            .get_job(job_id.as_str())
            .ok_or_else(|| RuntimeError::JobNotFound(job_id.to_string()))?;
        self.suspend_job(&job).await
    }

    pub(crate) async fn handle_job_cancel(
        &self,
        job_id: &JobId,
    ) -> Result<Vec<Event>, RuntimeError> {
        let job = self
            .get_job(job_id.as_str())
            .ok_or_else(|| RuntimeError::JobNotFound(job_id.to_string()))?;
        self.cancel_job(&job).await
    }

    pub(crate) async fn handle_workspace_drop(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Delete workspace via the standard effect (handles directory removal + state update)
        self.executor
            .execute(Effect::DeleteWorkspace { workspace_id: workspace_id.clone() })
            .await?;

        tracing::info!(workspace_id = %workspace_id, "deleted workspace");
        Ok(vec![])
    }

    /// Handle resume for shell step: re-run the command
    pub(crate) async fn handle_shell_resume(
        &self,
        job: &oj_core::Job,
        step: &str,
        _command: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Kill existing agent if any (defensive - shouldn't happen for shell steps)
        if let Some(agent_id) =
            job.step_history.iter().rfind(|r| r.name == step).and_then(|r| r.agent_id.as_ref())
        {
            let _ =
                self.executor.execute(Effect::KillAgent { agent_id: AgentId::new(agent_id) }).await;
        }

        // Re-run the shell command
        let execution_dir = job.execution_dir().to_path_buf();
        let job_id = JobId::new(&job.id);
        let result = self.start_step(&job_id, step, &job.vars, &execution_dir).await?;

        tracing::info!(job_id = %job.id, "re-running shell step");
        Ok(result)
    }

    pub(crate) async fn handle_shell_exited(
        &self,
        job_id: &JobId,
        step: &str,
        exit_code: i32,
        stdout: Option<&str>,
        stderr: Option<&str>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let job = self.require_job(job_id.as_str())?;

        // Verify we're in the expected step
        if job.step != step {
            tracing::warn!(
                job_id = %job_id,
                expected = step,
                actual = %job.step,
                "shell completed for unexpected step"
            );
            return Ok(vec![]);
        }

        // Write captured output before the status line
        if let Some(out) = stdout {
            self.logger.append_fenced(job_id.as_str(), step, "stdout", out);
        }
        if let Some(err) = stderr {
            self.logger.append_fenced(job_id.as_str(), step, "stderr", err);
        }

        if exit_code == 0 {
            self.logger.append(
                job_id.as_str(),
                step,
                &format!("shell completed (exit {})", exit_code),
            );
            self.advance_job(&job).await
        } else {
            self.logger.append(
                job_id.as_str(),
                step,
                &format!("shell failed (exit {})", exit_code),
            );
            self.fail_job(&job, &format!("shell exit code: {}", exit_code)).await
        }
    }

    /// Handle JobCreated: kick off workspace creation or start first step.
    ///
    /// Called when the event loop processes a `JobCreated` event emitted by
    /// `create_and_start_job()`. Reads workspace metadata from job vars to
    /// decide whether to build a `CreateWorkspace` effect (background task
    /// fires `WorkspaceReady` → `start_first_step()`) or start directly.
    ///
    /// Idempotent: no-op if the step is already past Pending.
    pub(crate) async fn handle_job_created(
        &self,
        job_id: &JobId,
    ) -> Result<Vec<Event>, RuntimeError> {
        let Some(job) = self.get_job(job_id.as_str()) else {
            return Ok(vec![]);
        };

        // Idempotency guard: skip if step already started (e.g. WAL replay)
        if job.step_status != StepStatus::Pending {
            return Ok(vec![]);
        }

        // Check if this job has a workspace to create
        if let Some(ws_id) = job.vars.get("source.id") {
            let workspace_type = job.vars.get("source.type").cloned();
            let is_worktree = workspace_type.as_deref() == Some("worktree");

            let (repo_root, branch, start_point) = if is_worktree {
                (
                    job.vars.get("source.repo_root").map(std::path::PathBuf::from),
                    job.vars.get("source.branch").cloned(),
                    Some(job.vars.get("source.ref").cloned().unwrap_or_else(|| "HEAD".to_string())),
                )
            } else {
                (None, None, None)
            };

            let ws_events = self
                .executor
                .execute_all(vec![Effect::CreateWorkspace {
                    workspace_id: WorkspaceId::new(ws_id),
                    path: job.cwd.clone(),
                    owner: OwnerId::Job(job_id.clone()),
                    workspace_type,
                    repo_root,
                    branch,
                    start_point,
                }])
                .await?;

            // WorkspaceCreated is returned immediately; the background task
            // will fire WorkspaceReady → start_first_step().
            return Ok(ws_events);
        }

        // No workspace — start the first step directly
        self.start_first_step(job_id, &job).await
    }

    /// Handle WorkspaceReady: workspace filesystem setup completed successfully.
    ///
    /// Looks up the owning job and calls start_first_step() to begin execution.
    /// Idempotent: no-op if the job is already past Pending (e.g. already running).
    pub(crate) async fn handle_workspace_ready(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Find the owning job via workspace record
        let owner_job_id = self.lock_state(|s| {
            s.workspaces.get(workspace_id.as_str()).and_then(|ws| ws.owner.as_job().cloned())
        });
        let Some(job_id) = owner_job_id else {
            return Ok(vec![]);
        };
        let Some(job) = self.get_job(job_id.as_str()) else {
            return Ok(vec![]);
        };

        // Guard: only start the step if it's still pending (idempotent)
        if job.step_status != StepStatus::Pending {
            return Ok(vec![]);
        }

        self.start_first_step(&job_id, &job).await
    }

    /// Shared helper: look up the first step from the runbook and start it.
    ///
    /// Used by both `handle_job_created` (no-workspace path) and
    /// `handle_workspace_ready` (after workspace setup completes).
    async fn start_first_step(
        &self,
        job_id: &JobId,
        job: &oj_core::Job,
    ) -> Result<Vec<Event>, RuntimeError> {
        let runbook = self.cached_runbook(&job.runbook_hash)?;
        let job_def = runbook
            .get_job(&job.kind)
            .ok_or_else(|| RuntimeError::JobDefNotFound(job.kind.clone()))?;

        let execution_dir = job.execution_dir().to_path_buf();
        if let Some(step) = job_def.first_step() {
            self.start_step(job_id, &step.name, &job.vars, &execution_dir).await
        } else {
            Ok(vec![])
        }
    }

    /// Handle WorkspaceFailed: workspace filesystem setup failed.
    ///
    /// Looks up the owning job and calls fail_job().
    /// Idempotent: no-op if the job is already terminal.
    pub(crate) async fn handle_workspace_failed(
        &self,
        workspace_id: &WorkspaceId,
        reason: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Find the owning job via workspace record
        let owner_job_id = self.lock_state(|s| {
            s.workspaces.get(workspace_id.as_str()).and_then(|ws| ws.owner.as_job().cloned())
        });
        let Some(job_id) = owner_job_id else {
            return Ok(vec![]);
        };
        let Some(job) = self.get_active_job(job_id.as_str()) else {
            return Ok(vec![]);
        };
        self.fail_job(&job, reason).await
    }

    /// Handle AgentSpawned: agent spawn completed successfully.
    ///
    /// Sets up the liveness timer and emits on_start notifications.
    /// This handler fires asynchronously after the background SpawnAgent task
    /// completes. Idempotent: no-op if the job/crew is already terminal.
    pub(crate) async fn handle_agent_spawned(
        &self,
        owner: &OwnerId,
    ) -> Result<Vec<Event>, RuntimeError> {
        let Some(run) = self.get_active_run(owner) else {
            // For jobs: terminal (e.g. cancelled during spawn) — kill the orphan agent
            if let OwnerId::Job(job_id) = owner {
                if let Some(job) = self.get_job(job_id.as_str()) {
                    if let Some(agent_id) = job
                        .step_history
                        .iter()
                        .rfind(|r| r.name == job.step)
                        .and_then(|r| r.agent_id.as_ref())
                    {
                        tracing::info!(
                            job_id = %job_id,
                            agent_id = %agent_id,
                            "agent_spawned: job terminal, killing orphan agent"
                        );
                        let _ = self
                            .executor
                            .execute(Effect::KillAgent { agent_id: AgentId::new(agent_id) })
                            .await;
                    }
                }
            }
            return Ok(vec![]);
        };

        // Set liveness timer
        self.executor
            .execute(Effect::SetTimer {
                id: TimerId::liveness(owner),
                duration: crate::engine::spawn::LIVENESS_INTERVAL,
            })
            .await?;

        // Emit on_start notification if configured
        let mut result_events = Vec::new();
        if let Ok(runbook) = self.cached_runbook(run.runbook_hash()) {
            if let Ok(agent_def) = run.resolve_agent_def(&runbook) {
                if let Some(effect) =
                    crate::engine::lifecycle::notify_on_start(run.as_ref(), &agent_def)
                {
                    if let Some(ev) = self.executor.execute(effect).await? {
                        result_events.push(ev);
                    }
                }
            }
        }

        Ok(result_events)
    }

    /// Handle AgentSpawnFailed: background agent spawn task failed.
    ///
    /// For job-owned agents: deregisters the agent mapping and fails the job.
    /// For crew: emits CrewUpdated::Failed.
    pub(crate) async fn handle_agent_spawn_failed(
        &self,
        agent_id: &AgentId,
        owner: &OwnerId,
        reason: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Deregister agent mapping (was registered before spawn was dispatched)
        self.agent_owners.lock().remove(agent_id);

        // Write error to agent log
        self.logger.append_agent_error(agent_id.as_str(), reason);

        match owner {
            OwnerId::Job(job_id) => {
                let Some(job) = self.get_active_job(job_id.as_str()) else {
                    return Ok(vec![]);
                };

                self.logger.append(
                    job_id.as_str(),
                    &job.step,
                    &format!("agent spawn failed: {}", reason),
                );
                self.fail_job(&job, reason).await
            }
            OwnerId::Crew(crew_id) => {
                tracing::error!(crew_id = %crew_id, error = %reason, "standalone agent spawn failed");

                let fail_event = Event::CrewUpdated {
                    id: crew_id.clone(),
                    status: CrewStatus::Failed,
                    reason: Some(reason.to_string()),
                };
                let result = self.executor.execute(Effect::Emit { event: fail_event }).await?;
                Ok(result.into_iter().collect())
            }
        }
    }

    /// Handle JobDeleted event with cascading cleanup.
    ///
    /// This is called when a job is explicitly deleted (e.g., via `oj agent prune`).
    /// It cleans up all associated resources:
    /// - Cancels all job-scoped timers
    /// - Deregisters agent→job mappings
    /// - Kills any running agents/sessions
    /// - Deletes associated workspaces
    ///
    /// All cleanup is best-effort: errors are logged but don't fail the deletion.
    pub(crate) async fn handle_job_deleted(
        &self,
        job_id: &JobId,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Snapshot job info BEFORE it gets deleted from state.
        // This handler runs before MaterializedState::apply_event.
        let job = self.get_job(job_id.as_str());

        // 1. Cancel all job-scoped timers using prefix match
        // Timer IDs are formatted as "type:job_id" (e.g., "liveness:abc123")
        let timer_prefix = format!(":{}", job_id.as_str());
        {
            let scheduler = self.executor.scheduler();
            let mut sched = scheduler.lock();
            // Cancel known timer types
            sched.cancel_timer(&format!("liveness{}", timer_prefix));
            sched.cancel_timer(&format!("exit-deferred{}", timer_prefix));
            sched.cancel_timer(&format!("idle-grace{}", timer_prefix));
            // Cancel any cooldown timers (dynamic suffixes like cooldown:abc123:exit:0)
            sched.cancel_timers_with_prefix(&format!("cooldown:{}", job_id.as_str()));
        }

        // The following cleanup depends on having job info
        let Some(job) = job else {
            return Ok(vec![]);
        };

        // 2. Collect agent IDs from step history to deregister
        let agent_ids: Vec<AgentId> =
            job.step_history.iter().filter_map(|r| r.agent_id.as_ref().map(AgentId::new)).collect();

        // 3. Deregister agent→job mappings (prevents stale watcher events)
        for agent_id in &agent_ids {
            self.agent_owners.lock().remove(agent_id);
        }

        // 4. Kill agents (this also stops their watchers)
        for agent_id in &agent_ids {
            let _ = self.executor.execute(Effect::KillAgent { agent_id: agent_id.clone() }).await;
        }

        // 5. Capture terminal + session log before killing session
        self.capture_before_kill_job(&job).await;

        // 6. Delete workspace if one exists
        let ws_id = job.workspace_id.clone().or_else(|| {
            self.lock_state(|s| {
                s.workspaces
                    .values()
                    .find(|ws| ws.owner == oj_core::OwnerId::Job(oj_core::JobId::new(&job.id)))
                    .map(|ws| oj_core::WorkspaceId::new(&ws.id))
            })
        });

        if let Some(ws_id) = ws_id {
            let exists = self.lock_state(|s| s.workspaces.contains_key(ws_id.as_str()));
            if exists {
                let _ = self
                    .executor
                    .execute(Effect::DeleteWorkspace { workspace_id: ws_id.clone() })
                    .await;
            }
        }

        tracing::info!(job_id = %job_id, "cascading cleanup for deleted job");
        Ok(vec![])
    }
}
