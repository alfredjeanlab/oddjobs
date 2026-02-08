// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job lifecycle event handling (resume, cancel, workspace, shell)

use super::super::Runtime;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{
    AgentId, Clock, Effect, Event, JobId, OwnerId, SessionId, StepOutcome, StepStatus, WorkspaceId,
};
use std::collections::HashMap;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
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
            Some(
                message
                    .unwrap_or("Please continue with the task.")
                    .to_string(),
            )
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
                    event: Event::JobAdvanced {
                        id: job_id.clone(),
                        step: resume_step.clone(),
                    },
                })
                .await?;
            result_events.extend(events);
        }

        // Persist var updates if any
        if !vars.is_empty() {
            self.executor
                .execute(Effect::Emit {
                    event: Event::JobUpdated {
                        id: JobId::new(&job.id),
                        vars: vars.clone(),
                    },
                })
                .await?;
        }

        // Merge vars for this resume operation
        let merged_inputs: HashMap<String, String> = job
            .vars
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .chain(vars.clone())
            .collect();

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
                tracing::warn!(
                    job_id = %job.id,
                    "resume --message ignored for shell steps"
                );
            }

            let command = step_def
                .shell_command()
                .ok_or_else(|| RuntimeError::InvalidRequest("no shell command in step".into()))?;

            let events = self
                .handle_shell_resume(&job, &resume_step, command)
                .await?;
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
            .execute(Effect::DeleteWorkspace {
                workspace_id: workspace_id.clone(),
            })
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
        // Kill existing session if any
        if let Some(session_id) = &job.session_id {
            let _ = self
                .executor
                .execute(Effect::KillSession {
                    session_id: SessionId::new(session_id),
                })
                .await;
        }

        // Re-run the shell command
        let execution_dir = self.execution_dir(job);
        let job_id = JobId::new(&job.id);
        let result = self
            .start_step(&job_id, step, &job.vars, &execution_dir)
            .await?;

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
            self.logger
                .append_fenced(job_id.as_str(), step, "stdout", out);
        }
        if let Some(err) = stderr {
            self.logger
                .append_fenced(job_id.as_str(), step, "stderr", err);
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
            self.fail_job(&job, &format!("shell exit code: {}", exit_code))
                .await
        }
    }

    /// Handle WorkspaceReady: workspace filesystem setup completed successfully.
    ///
    /// Looks up the owning job and calls start_step() to begin the first step.
    /// Idempotent: no-op if the job is already past Pending (e.g. already running).
    pub(crate) async fn handle_workspace_ready(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Find the owning job via workspace record
        let owner_job_id = self.lock_state(|s| {
            s.workspaces
                .get(workspace_id.as_str())
                .and_then(|ws| ws.owner.as_ref())
                .and_then(|owner| match owner {
                    OwnerId::Job(id) => Some(id.clone()),
                    _ => None,
                })
        });

        let Some(job_id) = owner_job_id else {
            tracing::debug!(
                workspace_id = %workspace_id,
                "workspace_ready: no owning job found"
            );
            return Ok(vec![]);
        };

        let Some(job) = self.get_job(job_id.as_str()) else {
            tracing::debug!(
                job_id = %job_id,
                "workspace_ready: job not found"
            );
            return Ok(vec![]);
        };

        // Guard: only start the step if it's still pending (idempotent)
        if job.step_status != StepStatus::Pending {
            tracing::debug!(
                job_id = %job_id,
                step_status = %job.step_status,
                "workspace_ready: step already started, skipping"
            );
            return Ok(vec![]);
        }

        let runbook = self.cached_runbook(&job.runbook_hash)?;
        let job_def = runbook
            .get_job(&job.kind)
            .ok_or_else(|| RuntimeError::JobDefNotFound(job.kind.clone()))?;

        let first_step_name = job_def.first_step().map(|p| p.name.clone());
        let execution_dir = self.execution_dir(&job);

        if let Some(step_name) = first_step_name {
            self.start_step(&job_id, &step_name, &job.vars, &execution_dir)
                .await
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
            s.workspaces
                .get(workspace_id.as_str())
                .and_then(|ws| ws.owner.as_ref())
                .and_then(|owner| match owner {
                    OwnerId::Job(id) => Some(id.clone()),
                    _ => None,
                })
        });

        let Some(job_id) = owner_job_id else {
            tracing::debug!(
                workspace_id = %workspace_id,
                "workspace_failed: no owning job found"
            );
            return Ok(vec![]);
        };

        let Some(job) = self.get_active_job(job_id.as_str()) else {
            tracing::debug!(
                job_id = %job_id,
                "workspace_failed: job not found or already terminal"
            );
            return Ok(vec![]);
        };

        self.fail_job(&job, reason).await
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
            tracing::debug!(job_id = %job_id, "job_deleted: job not found (already deleted or never existed)");
            return Ok(vec![]);
        };

        // 2. Collect agent IDs from step history to deregister
        let agent_ids: Vec<AgentId> = job
            .step_history
            .iter()
            .filter_map(|r| r.agent_id.as_ref().map(AgentId::new))
            .collect();

        // 3. Deregister agent→job mappings (prevents stale watcher events)
        for agent_id in &agent_ids {
            self.deregister_agent(agent_id);
        }

        // 4. Kill agents (this also stops their watchers)
        for agent_id in &agent_ids {
            if let Err(e) = self
                .executor
                .execute(Effect::KillAgent {
                    agent_id: agent_id.clone(),
                })
                .await
            {
                tracing::debug!(
                    job_id = %job_id,
                    agent_id = %agent_id,
                    error = %e,
                    "job_deleted: failed to kill agent (may already be dead)"
                );
            }
        }

        // 5. Capture terminal + session log before killing session
        self.capture_before_kill_job(&job).await;

        // 6. Kill session as fallback (in case agent kill didn't cover it)
        if let Some(session_id) = &job.session_id {
            let sid = SessionId::new(session_id);
            if let Err(e) = self
                .executor
                .execute(Effect::KillSession {
                    session_id: sid.clone(),
                })
                .await
            {
                tracing::debug!(
                    job_id = %job_id,
                    session_id = %session_id,
                    error = %e,
                    "job_deleted: failed to kill session (may already be dead)"
                );
            }
            // Emit SessionDeleted event so state is consistent
            let _ = self
                .executor
                .execute(Effect::Emit {
                    event: Event::SessionDeleted { id: sid },
                })
                .await;
        }

        // 7. Delete workspace if one exists
        let ws_id = job.workspace_id.clone().or_else(|| {
            self.lock_state(|s| {
                s.workspaces
                    .values()
                    .find(|ws| {
                        ws.owner.as_ref()
                            == Some(&oj_core::OwnerId::Job(oj_core::JobId::new(&job.id)))
                    })
                    .map(|ws| oj_core::WorkspaceId::new(&ws.id))
            })
        });

        if let Some(ws_id) = ws_id {
            let exists = self.lock_state(|s| s.workspaces.contains_key(ws_id.as_str()));
            if exists {
                if let Err(e) = self
                    .executor
                    .execute(Effect::DeleteWorkspace {
                        workspace_id: ws_id.clone(),
                    })
                    .await
                {
                    tracing::debug!(
                        job_id = %job_id,
                        workspace_id = %ws_id,
                        error = %e,
                        "job_deleted: failed to delete workspace"
                    );
                }
            }
        }

        tracing::info!(job_id = %job_id, "cascading cleanup for deleted job");
        Ok(vec![])
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
