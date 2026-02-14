// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job lifecycle management

use super::Runtime;
use crate::engine::error::RuntimeError;
use crate::engine::steps;
use oj_core::{Clock, Effect, Event, Job, JobId, TimerId};
use oj_runbook::{NotifyConfig, RunDirective};
use std::collections::HashMap;
use std::path::Path;

impl<C: Clock> Runtime<C> {
    pub(crate) async fn start_step(
        &self,
        job_id: &JobId,
        step_name: &str,
        input: &HashMap<String, String>,
        workspace_path: &Path,
    ) -> Result<Vec<Event>, RuntimeError> {
        let job = self.require_job(job_id.as_str())?;
        let runbook = self.cached_runbook(&job.runbook_hash)?;

        let job_def = runbook
            .get_job(&job.kind)
            .ok_or_else(|| RuntimeError::JobDefNotFound(job.kind.clone()))?;

        // Circuit breaker: prevent runaway retry cycles by limiting how many
        // times any single step can be entered. step_visits is incremented
        // when JobAdvanced is applied, so the count here is already
        // current for this visit.
        let visits = job.get_step_visits(step_name);
        if visits > oj_core::job::MAX_STEP_VISITS {
            let error = format!(
                "circuit breaker: step '{}' entered {} times (limit {})",
                step_name,
                visits,
                oj_core::job::MAX_STEP_VISITS,
            );
            tracing::warn!(job_id = %job.id, %error);
            self.logger.append(job_id.as_str(), step_name, &error);
            let effects = steps::failure_effects(&job, &error);
            let mut result_events = self.executor.execute_all(effects).await?;
            self.breadcrumb.delete(&job.id);

            // Emit on_fail notification for the terminal failure
            result_events.extend(
                self.emit_notify(&job, &job_def.notify, job_def.notify.on_fail.as_ref()).await?,
            );

            return Ok(result_events);
        }

        let step_def = job_def
            .get_step(step_name)
            .ok_or_else(|| RuntimeError::JobNotFound(format!("step {} not found", step_name)))?;

        let mut result_events = Vec::new();

        // Mark step as running
        let effects = steps::step_start_effects(job_id, step_name);
        result_events.extend(self.executor.execute_all(effects).await?);
        self.logger.append(job_id.as_str(), step_name, "step started");

        // Write breadcrumb after step status change (captures agent info)
        if let Some(job) = self.get_job(job_id.as_str()) {
            self.breadcrumb.write(&job);
        }

        // Dispatch based on run directive
        match &step_def.run {
            RunDirective::Shell(cmd) => {
                // Build template variables — project bare keys under "var." prefix.
                // Values are escaped by interpolate_shell() during substitution.
                let mut vars = crate::engine::vars::namespace_vars(input);
                vars.insert("job_id".to_string(), job_id.to_string());
                vars.insert("name".to_string(), job.name.clone());
                vars.insert("workspace".to_string(), workspace_path.display().to_string());

                let command = oj_runbook::interpolate_shell(cmd, &vars);
                self.logger.append(
                    job_id.as_str(),
                    step_name,
                    &format!("shell (cwd: {}): {}", workspace_path.display(), command),
                );

                let mut shell_env = HashMap::new();
                if !job.project.is_empty() {
                    shell_env.insert("OJ_PROJECT".to_string(), job.project.clone());
                }

                let effects = vec![Effect::Shell {
                    owner: Some(oj_core::OwnerId::Job(job_id.clone())),
                    step: step_name.to_string(),
                    command,
                    cwd: workspace_path.to_path_buf(),
                    env: shell_env,
                    container: None,
                }];

                result_events.extend(self.executor.execute_all(effects).await?);
            }

            RunDirective::Agent { agent, .. } => {
                result_events.extend(self.spawn_agent(job_id, agent, input).await?);
            }

            RunDirective::Job { job } => {
                return Err(Self::invalid_directive(
                    &format!("step {step_name}"),
                    "nested job",
                    job,
                ));
            }
        }

        Ok(result_events)
    }

    pub(crate) async fn advance_job(&self, job: &Job) -> Result<Vec<Event>, RuntimeError> {
        // If current step is terminal (done/failed), complete the job
        // This handles the case where a "done" step has a run command that just finished
        if job.is_terminal() {
            return self.complete_job(job).await;
        }

        if self.is_agent_step(job) {
            self.finalize_agent_step(job).await?;
        }

        let runbook = self.cached_runbook(&job.runbook_hash)?;
        let job_def = runbook.get_job(&job.kind);
        let current_step_def = job_def.as_ref().and_then(|p| p.get_step(&job.step));
        let job_id = JobId::from_string(&job.id);

        // Mark current step as completed so that JobAdvanced sees
        // step_status == Completed and correctly resets attempts.
        // (Without this, an agent exiting non-zero with on_dead="done" would
        // leave step_status == Failed, causing attempts to carry over.)
        self.executor
            .execute(Effect::Emit {
                event: Event::StepCompleted { job_id: job_id.clone(), step: job.step.clone() },
            })
            .await?;

        // Determine next step: explicit on_done > complete
        // Steps without on_done complete the job (same as on_fail requiring explicit targets)
        let next_transition = current_step_def.and_then(|p| p.on_done.clone());

        let mut result_events = Vec::new();

        match next_transition {
            Some(transition) => {
                let next_step = transition.step_name();
                self.logger.append(&job.id, &job.step, &format!("advancing to {}", next_step));
                let effects = steps::step_transition_effects(job, next_step);
                result_events.extend(self.executor.execute_all(effects).await?);

                let has_step_def = job_def.as_ref().and_then(|p| p.get_step(next_step)).is_some();
                let is_terminal = next_step == "done" || next_step == "failed";

                if !has_step_def && is_terminal {
                    result_events.extend(self.complete_job(job).await?);
                } else {
                    result_events.extend(
                        self.start_step(&job_id, next_step, &job.vars, job.execution_dir()).await?,
                    );
                }
            }
            None => {
                let job_on_done = job_def.as_ref().and_then(|p| p.on_done.clone());
                if let Some(ref on_done) = job_on_done {
                    let on_done_step = on_done.step_name();
                    if job.step != on_done_step {
                        // Job-level on_done: route to that step instead of completing
                        self.logger.append(
                            &job.id,
                            &job.step,
                            &format!("job on_done: advancing to {}", on_done_step),
                        );
                        let effects = steps::step_transition_effects(job, on_done_step);
                        result_events.extend(self.executor.execute_all(effects).await?);
                        result_events.extend(
                            self.start_step(&job_id, on_done_step, &job.vars, job.execution_dir())
                                .await?,
                        );
                    } else {
                        // Already at on_done target; complete normally
                        let effects = steps::step_transition_effects(job, "done");
                        result_events.extend(self.executor.execute_all(effects).await?);
                        result_events.extend(self.complete_job(job).await?);
                    }
                } else if job.failing {
                    // On-fail cleanup step completed; go to terminal "failed"
                    result_events.extend(self.terminate_failed_job(job).await?);
                } else if job.cancelling {
                    // Cancel cleanup step completed; go to terminal "cancelled"
                    result_events.extend(self.terminate_cancelled_job(job).await?);
                } else {
                    let effects = steps::step_transition_effects(job, "done");
                    result_events.extend(self.executor.execute_all(effects).await?);
                    result_events.extend(self.complete_job(job).await?);
                }
            }
        }

        Ok(result_events)
    }

    pub(crate) async fn fail_job(
        &self,
        job: &Job,
        error: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        if self.is_agent_step(job) {
            self.finalize_agent_step(job).await?;
        }

        let runbook = self.cached_runbook(&job.runbook_hash)?;
        let job_def = runbook.get_job(&job.kind);
        let current_step_def = job_def.as_ref().and_then(|p| p.get_step(&job.step));
        let on_fail = current_step_def.and_then(|p| p.on_fail.as_ref());
        let job_id = JobId::from_string(&job.id);

        self.logger.append(&job.id, &job.step, &format!("job failed: {}", error));

        let mut result_events = Vec::new();

        if let Some(on_fail) = on_fail {
            let on_fail_step = on_fail.step_name();
            // Mark job as failing so advance_job() routes to failed terminal after cleanup
            result_events.extend(
                self.executor
                    .execute(Effect::Emit { event: Event::JobFailing { id: job_id.clone() } })
                    .await?,
            );
            let effects = steps::failure_transition_effects(job, on_fail_step, error);
            result_events.extend(self.executor.execute_all(effects).await?);
            result_events.extend(
                self.start_step(&job_id, on_fail_step, &job.vars, job.execution_dir()).await?,
            );
        } else if let Some(ref job_on_fail) = job_def.as_ref().and_then(|p| p.on_fail.clone()) {
            let on_fail_step = job_on_fail.step_name();
            if job.step != on_fail_step {
                // Job-level on_fail: route to that step
                self.logger.append(
                    &job.id,
                    &job.step,
                    &format!("job on_fail: advancing to {}", on_fail_step),
                );
                // Mark job as failing so advance_job() routes to failed terminal after cleanup
                result_events.extend(
                    self.executor
                        .execute(Effect::Emit { event: Event::JobFailing { id: job_id.clone() } })
                        .await?,
                );
                let effects = steps::failure_transition_effects(job, on_fail_step, error);
                result_events.extend(self.executor.execute_all(effects).await?);
                result_events.extend(
                    self.start_step(&job_id, on_fail_step, &job.vars, job.execution_dir()).await?,
                );
            } else {
                // Already at the job on_fail target; terminal failure
                let effects = steps::failure_effects(job, error);
                result_events.extend(self.executor.execute_all(effects).await?);
                self.breadcrumb.delete(&job.id);
                // Update queue item status immediately (don't rely on event loop)
                result_events.extend(self.check_worker_job_complete(&job_id, "failed").await?);
            }
        } else {
            // Terminal failure — no on_fail handler
            let effects = steps::failure_effects(job, error);
            result_events.extend(self.executor.execute_all(effects).await?);
            self.breadcrumb.delete(&job.id);

            // Update queue item status immediately (don't rely on event loop)
            result_events.extend(self.check_worker_job_complete(&job_id, "failed").await?);

            // Emit on_fail notification only on terminal failure (not on_fail transition)
            if let Some(job_def) = job_def.as_ref() {
                result_events.extend(
                    self.emit_notify(job, &job_def.notify, job_def.notify.on_fail.as_ref()).await?,
                );
            }
        }

        Ok(result_events)
    }

    pub(crate) async fn complete_job(&self, job: &Job) -> Result<Vec<Event>, RuntimeError> {
        self.logger.append(&job.id, &job.step, "job completed");
        self.breadcrumb.delete(&job.id);
        let mut effects = steps::completion_effects(job);

        // Clean up workspaces on successful completion
        effects.extend(self.workspace_cleanup_effects(job));

        let mut result_events = self.executor.execute_all(effects).await?;

        // Update queue item status immediately (don't rely on event loop)
        let job_id = JobId::from_string(&job.id);
        result_events.extend(self.check_worker_job_complete(&job_id, "done").await?);

        // Emit on_done notification if configured
        if let Ok(runbook) = self.cached_runbook(&job.runbook_hash) {
            if let Some(job_def) = runbook.get_job(&job.kind) {
                result_events.extend(
                    self.emit_notify(job, &job_def.notify, job_def.notify.on_done.as_ref()).await?,
                );
            }
        }

        Ok(result_events)
    }

    /// Emit a notification effect if a notify message template is configured.
    pub(crate) async fn emit_notify(
        &self,
        job: &Job,
        notify: &NotifyConfig,
        message_template: Option<&String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        if let Some(template) = message_template {
            let mut vars = crate::engine::vars::namespace_vars(&job.vars);
            vars.insert("job_id".to_string(), job.id.clone());
            vars.insert("name".to_string(), job.name.clone());
            if let Some(err) = &job.error {
                vars.insert("error".to_string(), err.clone());
            }

            let message = NotifyConfig::render(template, &vars);
            let event =
                self.executor.execute(Effect::Notify { title: job.name.clone(), message }).await?;
            return Ok(event.into_iter().collect());
        }
        let _ = notify; // silence unused warning when no template
        Ok(vec![])
    }

    /// Suspend a running job.
    ///
    /// Kills the agent session and transitions to the "suspended" terminal state.
    /// Unlike cancellation, does NOT clean up the workspace — preserves everything
    /// for later resume. No on_cancel/on_suspend routing; goes straight to terminal.
    pub(crate) async fn suspend_job(&self, job: &Job) -> Result<Vec<Event>, RuntimeError> {
        // Already terminal — no-op
        if job.is_terminal() {
            tracing::info!(job_id = %job.id, "suspend: job already terminal");
            return Ok(vec![]);
        }

        // If already suspending, don't re-suspend
        if job.suspending {
            tracing::info!(job_id = %job.id, "suspend: already suspending, ignoring");
            return Ok(vec![]);
        }

        if self.is_agent_step(job) {
            self.finalize_agent_step(job).await?;
        }

        // Go directly to suspended terminal — no cleanup step routing
        let effects = steps::suspension_effects(job);
        // NOTE: no workspace cleanup — preserve for resume
        let result_events = self.executor.execute_all(effects).await?;
        self.breadcrumb.delete(&job.id);

        tracing::info!(job_id = %job.id, "suspended job");
        Ok(result_events)
    }

    /// Cancel a running job.
    ///
    /// If the current step (or job) has `on_cancel` configured, routes to
    /// that cleanup step instead of going straight to terminal. The cleanup step
    /// is non-cancellable — re-cancellation while `cancelling` is true is a no-op.
    pub(crate) async fn cancel_job(&self, job: &Job) -> Result<Vec<Event>, RuntimeError> {
        // Already terminal — no-op
        if job.is_terminal() {
            tracing::info!(job_id = %job.id, "cancel: job already terminal");
            return Ok(vec![]);
        }

        // If already running a cancel cleanup step, don't re-cancel — let it finish
        if job.cancelling {
            tracing::info!(job_id = %job.id, "cancel: already running cleanup, ignoring");
            return Ok(vec![]);
        }

        if self.is_agent_step(job) {
            self.finalize_agent_step(job).await?;
        }

        let runbook = self.cached_runbook(&job.runbook_hash)?;
        let job_def = runbook.get_job(&job.kind);
        let current_step_def = job_def.as_ref().and_then(|p| p.get_step(&job.step));
        let on_cancel = current_step_def.and_then(|s| s.on_cancel.as_ref());
        let job_id = JobId::from_string(&job.id);

        let mut result_events = Vec::new();

        if let Some(on_cancel) = on_cancel {
            // Step-level on_cancel: route to cleanup step
            let target = on_cancel.step_name();
            result_events.extend(
                self.executor
                    .execute(Effect::Emit { event: Event::JobCancelling { id: job_id.clone() } })
                    .await?,
            );
            let effects = steps::cancellation_transition_effects(job, target);
            result_events.extend(self.executor.execute_all(effects).await?);
            result_events
                .extend(self.start_step(&job_id, target, &job.vars, job.execution_dir()).await?);
        } else if let Some(ref job_on_cancel) = job_def.as_ref().and_then(|p| p.on_cancel.clone()) {
            // Job-level on_cancel fallback
            let target = job_on_cancel.step_name();
            if job.step != target {
                result_events.extend(
                    self.executor
                        .execute(Effect::Emit {
                            event: Event::JobCancelling { id: job_id.clone() },
                        })
                        .await?,
                );
                let effects = steps::cancellation_transition_effects(job, target);
                result_events.extend(self.executor.execute_all(effects).await?);
                result_events.extend(
                    self.start_step(&job_id, target, &job.vars, job.execution_dir()).await?,
                );
            } else {
                // Already at the cancel target; go terminal
                result_events.extend(self.terminate_cancelled_job(job).await?);
            }
        } else {
            // No on_cancel configured; terminal cancellation
            result_events.extend(self.terminate_cancelled_job(job).await?);
        }

        tracing::info!(job_id = %job.id, "cancelled job");
        Ok(result_events)
    }

    /// Whether the job's current step is an agent step.
    fn is_agent_step(&self, job: &Job) -> bool {
        self.cached_runbook(&job.runbook_hash)
            .ok()
            .and_then(|rb| rb.get_job(&job.kind)?.get_step(&job.step).cloned())
            .map(|s| matches!(&s.run, RunDirective::Agent { .. }))
            .unwrap_or(false)
    }

    /// Build workspace cleanup effects for a job (if it has a workspace).
    fn workspace_cleanup_effects(&self, job: &Job) -> Vec<Effect> {
        let job_owner: oj_core::OwnerId = oj_core::JobId::from_string(&job.id).into();
        let ws_id = job.workspace_id.clone().or_else(|| {
            self.lock_state(|s| {
                s.workspaces
                    .values()
                    .find(|ws| ws.owner == job_owner)
                    .map(|ws| oj_core::WorkspaceId::from_string(&ws.id))
            })
        });
        if let Some(ws_id) = ws_id {
            if self.lock_state(|s| s.workspaces.contains_key(ws_id.as_str())) {
                return vec![Effect::DeleteWorkspace { workspace_id: ws_id }];
            }
        }
        vec![]
    }

    /// Terminal failure after on_fail cleanup: emit failure effects, update queue.
    async fn terminate_failed_job(&self, job: &Job) -> Result<Vec<Event>, RuntimeError> {
        let error = job.error.as_deref().unwrap_or("on_fail cleanup completed");
        self.logger.append(
            &job.id,
            &job.step,
            &format!("on_fail cleanup done, failing job: {}", error),
        );
        let effects = steps::failure_after_cleanup_effects(job, error);
        let mut result_events = self.executor.execute_all(effects).await?;
        self.breadcrumb.delete(&job.id);
        let job_id = JobId::from_string(&job.id);
        result_events.extend(self.check_worker_job_complete(&job_id, "failed").await?);

        // Emit on_fail notification on terminal failure
        if let Ok(runbook) = self.cached_runbook(&job.runbook_hash) {
            if let Some(job_def) = runbook.get_job(&job.kind) {
                result_events.extend(
                    self.emit_notify(job, &job_def.notify, job_def.notify.on_fail.as_ref()).await?,
                );
            }
        }

        Ok(result_events)
    }

    /// Terminal cancellation: emit cancel effects, clean up workspace, update queue.
    async fn terminate_cancelled_job(&self, job: &Job) -> Result<Vec<Event>, RuntimeError> {
        let mut effects = steps::cancellation_effects(job);
        effects.extend(self.workspace_cleanup_effects(job));
        let mut result_events = self.executor.execute_all(effects).await?;
        self.breadcrumb.delete(&job.id);
        let job_id = JobId::from_string(&job.id);
        result_events.extend(self.check_worker_job_complete(&job_id, "cancelled").await?);
        Ok(result_events)
    }

    /// Clean up when leaving an agent step: cancel timers, deregister agent
    /// mapping, capture terminal output, and kill the agent process.
    async fn finalize_agent_step(&self, job: &Job) -> Result<(), RuntimeError> {
        let job_id = JobId::from_string(&job.id);
        self.executor.execute(Effect::CancelTimer { id: TimerId::liveness(&job_id) }).await?;
        self.executor.execute(Effect::CancelTimer { id: TimerId::exit_deferred(&job_id) }).await?;

        if let Some(agent_id) =
            job.step_history.iter().rfind(|r| r.name == job.step).and_then(|r| r.agent_id.as_ref())
        {
            self.agent_owners.lock().remove(&oj_core::AgentId::from_string(agent_id));
        }

        self.capture_before_kill_job(job).await;

        // Kill agent via AgentAdapter (handles session cleanup internally)
        if let Some(agent_id) =
            job.step_history.iter().rfind(|r| r.name == job.step).and_then(|r| r.agent_id.as_ref())
        {
            self.executor
                .execute(Effect::KillAgent { agent_id: oj_core::AgentId::from_string(agent_id) })
                .await?;
        }
        Ok(())
    }
}
