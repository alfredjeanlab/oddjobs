// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Pipeline lifecycle management

use super::Runtime;
use crate::error::RuntimeError;
use crate::steps;
use oj_adapters::{AgentAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, Pipeline, PipelineId, SessionId, TimerId};
use oj_runbook::{NotifyConfig, RunDirective};
use std::collections::HashMap;
use std::path::Path;

impl<S, A, C> Runtime<S, A, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    C: Clock,
{
    pub(crate) async fn start_step(
        &self,
        pipeline_id: &PipelineId,
        step_name: &str,
        input: &HashMap<String, String>,
        workspace_path: &Path,
    ) -> Result<Vec<Event>, RuntimeError> {
        let pipeline = self.require_pipeline(pipeline_id.as_str())?;
        let runbook = self.cached_runbook(&pipeline.runbook_hash)?;

        let pipeline_def = runbook
            .get_pipeline(&pipeline.kind)
            .ok_or_else(|| RuntimeError::PipelineDefNotFound(pipeline.kind.clone()))?;

        let step_def = pipeline_def.get_step(step_name).ok_or_else(|| {
            RuntimeError::PipelineNotFound(format!("step {} not found", step_name))
        })?;

        let mut result_events = Vec::new();

        // Mark step as running
        let effects = steps::step_start_effects(pipeline_id, step_name);
        result_events.extend(self.executor.execute_all(effects).await?);
        self.logger
            .append(pipeline_id.as_str(), step_name, "step started");

        // Dispatch based on run directive
        match &step_def.run {
            RunDirective::Shell(cmd) => {
                // Build template variables
                // Namespace pipeline vars under "var." prefix (matching monitor.rs)
                // Values are escaped by interpolate_shell() during substitution
                let mut vars: HashMap<String, String> = input
                    .iter()
                    .map(|(k, v)| (format!("var.{}", k), v.clone()))
                    .collect();
                vars.insert("pipeline_id".to_string(), pipeline_id.to_string());
                vars.insert("name".to_string(), pipeline.name.clone());
                vars.insert(
                    "workspace".to_string(),
                    workspace_path.display().to_string(),
                );
                // Expose workspace.*, invoke.*, and local.* variables at top level for shell interpolation
                for (key, val) in input.iter() {
                    if key.starts_with("workspace.")
                        || key.starts_with("invoke.")
                        || key.starts_with("local.")
                    {
                        vars.insert(key.clone(), val.clone());
                    }
                }

                let command = oj_runbook::interpolate_shell(cmd, &vars);
                self.logger.append(
                    pipeline_id.as_str(),
                    step_name,
                    &format!("shell (cwd: {}): {}", workspace_path.display(), command),
                );

                let mut shell_env = HashMap::new();
                if !pipeline.namespace.is_empty() {
                    shell_env.insert("OJ_NAMESPACE".to_string(), pipeline.namespace.clone());
                }

                let effects = vec![Effect::Shell {
                    pipeline_id: pipeline_id.clone(),
                    step: step_name.to_string(),
                    command,
                    cwd: workspace_path.to_path_buf(),
                    env: shell_env,
                }];

                result_events.extend(self.executor.execute_all(effects).await?);
            }

            RunDirective::Agent { agent } => {
                result_events.extend(self.spawn_agent(pipeline_id, agent, input).await?);
            }

            RunDirective::Pipeline { pipeline } => {
                return Err(Self::invalid_directive(
                    &format!("step {step_name}"),
                    "nested pipeline",
                    pipeline,
                ));
            }
        }

        Ok(result_events)
    }

    pub(crate) async fn advance_pipeline(
        &self,
        pipeline: &Pipeline,
    ) -> Result<Vec<Event>, RuntimeError> {
        // If current step is terminal (done/failed), complete the pipeline
        // This handles the case where a "done" step has a run command that just finished
        if pipeline.is_terminal() {
            return self.complete_pipeline(pipeline).await;
        }

        let runbook = self.cached_runbook(&pipeline.runbook_hash)?;
        let pipeline_def = runbook.get_pipeline(&pipeline.kind);
        let current_step_def = pipeline_def
            .as_ref()
            .and_then(|p| p.get_step(&pipeline.step));

        // Cancel session monitor timer when leaving an agent step
        let current_is_agent = current_step_def
            .map(|s| matches!(&s.run, RunDirective::Agent { .. }))
            .unwrap_or(false);
        let pipeline_id = PipelineId::new(&pipeline.id);
        if current_is_agent {
            self.executor
                .execute(Effect::CancelTimer {
                    id: TimerId::liveness(&pipeline_id),
                })
                .await?;
            self.executor
                .execute(Effect::CancelTimer {
                    id: TimerId::exit_deferred(&pipeline_id),
                })
                .await?;

            // Deregister the agent→pipeline mapping so stale watcher events
            // from the old agent are dropped as unknown.
            if let Some(agent_id) = pipeline
                .step_history
                .iter()
                .rfind(|r| r.name == pipeline.step)
                .and_then(|r| r.agent_id.as_ref())
            {
                self.agent_pipelines
                    .lock()
                    .remove(&oj_core::AgentId::new(agent_id));
            }

            // Kill the agent's tmux session before moving to the next step
            if let Some(session_id) = &pipeline.session_id {
                let sid = SessionId::new(session_id);
                self.executor
                    .execute(Effect::KillSession {
                        session_id: sid.clone(),
                    })
                    .await?;
                self.executor
                    .execute(Effect::Emit {
                        event: Event::SessionDeleted { id: sid },
                    })
                    .await?;
            }
        }

        // Determine next step: explicit on_done > complete
        // Steps without on_done complete the pipeline (same as on_fail requiring explicit targets)
        let next_transition = current_step_def.and_then(|p| p.on_done.clone());

        let mut result_events = Vec::new();

        match next_transition {
            Some(transition) => {
                let next_step = transition.step_name();
                self.logger.append(
                    &pipeline.id,
                    &pipeline.step,
                    &format!("advancing to {}", next_step),
                );
                let effects = steps::step_transition_effects(pipeline, next_step);
                result_events.extend(self.executor.execute_all(effects).await?);

                let has_step_def = pipeline_def
                    .as_ref()
                    .and_then(|p| p.get_step(next_step))
                    .is_some();
                let is_terminal = next_step == "done" || next_step == "failed";

                if !has_step_def && is_terminal {
                    result_events.extend(self.complete_pipeline(pipeline).await?);
                } else {
                    result_events.extend(
                        self.start_step(
                            &pipeline_id,
                            next_step,
                            &pipeline.vars,
                            &self.execution_dir(pipeline),
                        )
                        .await?,
                    );
                }
            }
            None => {
                let pipeline_on_done = pipeline_def.as_ref().and_then(|p| p.on_done.clone());
                if let Some(ref on_done) = pipeline_on_done {
                    let on_done_step = on_done.step_name();
                    if pipeline.step != on_done_step {
                        // Pipeline-level on_done: route to that step instead of completing
                        self.logger.append(
                            &pipeline.id,
                            &pipeline.step,
                            &format!("pipeline on_done: advancing to {}", on_done_step),
                        );
                        let effects = steps::step_transition_effects(pipeline, on_done_step);
                        result_events.extend(self.executor.execute_all(effects).await?);
                        result_events.extend(
                            self.start_step(
                                &pipeline_id,
                                on_done_step,
                                &pipeline.vars,
                                &self.execution_dir(pipeline),
                            )
                            .await?,
                        );
                    } else {
                        // Already at on_done target; complete normally
                        let effects = steps::step_transition_effects(pipeline, "done");
                        result_events.extend(self.executor.execute_all(effects).await?);
                        result_events.extend(self.complete_pipeline(pipeline).await?);
                    }
                } else {
                    let effects = steps::step_transition_effects(pipeline, "done");
                    result_events.extend(self.executor.execute_all(effects).await?);
                    result_events.extend(self.complete_pipeline(pipeline).await?);
                }
            }
        }

        Ok(result_events)
    }

    pub(crate) async fn fail_pipeline(
        &self,
        pipeline: &Pipeline,
        error: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let runbook = self.cached_runbook(&pipeline.runbook_hash)?;
        let pipeline_def = runbook.get_pipeline(&pipeline.kind);
        let current_step_def = pipeline_def
            .as_ref()
            .and_then(|p| p.get_step(&pipeline.step));
        let on_fail = current_step_def.and_then(|p| p.on_fail.as_ref());

        // Cancel session monitor timers when leaving an agent step
        let current_is_agent = current_step_def
            .map(|s| matches!(&s.run, RunDirective::Agent { .. }))
            .unwrap_or(false);
        let pipeline_id = PipelineId::new(&pipeline.id);
        if current_is_agent {
            self.executor
                .execute(Effect::CancelTimer {
                    id: TimerId::liveness(&pipeline_id),
                })
                .await?;
            self.executor
                .execute(Effect::CancelTimer {
                    id: TimerId::exit_deferred(&pipeline_id),
                })
                .await?;

            // Deregister the agent→pipeline mapping so stale watcher events
            // from the old agent are dropped as unknown.
            if let Some(agent_id) = pipeline
                .step_history
                .iter()
                .rfind(|r| r.name == pipeline.step)
                .and_then(|r| r.agent_id.as_ref())
            {
                self.agent_pipelines
                    .lock()
                    .remove(&oj_core::AgentId::new(agent_id));
            }

            // Kill the agent's tmux session before moving to the failure step
            if let Some(session_id) = &pipeline.session_id {
                let sid = SessionId::new(session_id);
                self.executor
                    .execute(Effect::KillSession {
                        session_id: sid.clone(),
                    })
                    .await?;
                self.executor
                    .execute(Effect::Emit {
                        event: Event::SessionDeleted { id: sid },
                    })
                    .await?;
            }
        }

        self.logger.append(
            &pipeline.id,
            &pipeline.step,
            &format!("pipeline failed: {}", error),
        );

        let mut result_events = Vec::new();

        if let Some(on_fail) = on_fail {
            let on_fail_step = on_fail.step_name();
            let effects = steps::failure_transition_effects(pipeline, on_fail_step, error);
            result_events.extend(self.executor.execute_all(effects).await?);
            result_events.extend(
                self.start_step(
                    &pipeline_id,
                    on_fail_step,
                    &pipeline.vars,
                    &self.execution_dir(pipeline),
                )
                .await?,
            );
        } else if let Some(ref pipeline_on_fail) =
            pipeline_def.as_ref().and_then(|p| p.on_fail.clone())
        {
            let on_fail_step = pipeline_on_fail.step_name();
            if pipeline.step != on_fail_step {
                // Pipeline-level on_fail: route to that step
                self.logger.append(
                    &pipeline.id,
                    &pipeline.step,
                    &format!("pipeline on_fail: advancing to {}", on_fail_step),
                );
                let effects = steps::failure_transition_effects(pipeline, on_fail_step, error);
                result_events.extend(self.executor.execute_all(effects).await?);
                result_events.extend(
                    self.start_step(
                        &pipeline_id,
                        on_fail_step,
                        &pipeline.vars,
                        &self.execution_dir(pipeline),
                    )
                    .await?,
                );
            } else {
                // Already at the pipeline on_fail target; fail normally
                let effects = steps::failure_effects(pipeline, error);
                result_events.extend(self.executor.execute_all(effects).await?);
            }
        } else {
            let effects = steps::failure_effects(pipeline, error);
            result_events.extend(self.executor.execute_all(effects).await?);

            // Emit on_fail notification only on terminal failure (not on_fail transition)
            if let Some(pipeline_def) = pipeline_def.as_ref() {
                result_events.extend(
                    self.emit_notify(
                        pipeline,
                        &pipeline_def.notify,
                        pipeline_def.notify.on_fail.as_ref(),
                    )
                    .await?,
                );
            }
        }

        Ok(result_events)
    }

    pub(crate) async fn complete_pipeline(
        &self,
        pipeline: &Pipeline,
    ) -> Result<Vec<Event>, RuntimeError> {
        self.logger
            .append(&pipeline.id, &pipeline.step, "pipeline completed");
        let mut effects = steps::completion_effects(pipeline);

        // Clean up ephemeral workspaces on successful completion
        if let Some(ws_id) = &pipeline.workspace_id {
            let is_ephemeral = self.lock_state(|state| {
                state
                    .workspaces
                    .get(ws_id.as_str())
                    .map(|ws| ws.mode == oj_storage::WorkspaceMode::Ephemeral)
                    .unwrap_or(false)
            });
            if is_ephemeral {
                effects.push(Effect::DeleteWorkspace {
                    workspace_id: ws_id.clone(),
                });
            }
        }

        let mut result_events = self.executor.execute_all(effects).await?;

        // Emit on_done notification if configured
        if let Ok(runbook) = self.cached_runbook(&pipeline.runbook_hash) {
            if let Some(pipeline_def) = runbook.get_pipeline(&pipeline.kind) {
                result_events.extend(
                    self.emit_notify(
                        pipeline,
                        &pipeline_def.notify,
                        pipeline_def.notify.on_done.as_ref(),
                    )
                    .await?,
                );
            }
        }

        Ok(result_events)
    }

    /// Emit a notification effect if a notify message template is configured.
    pub(crate) async fn emit_notify(
        &self,
        pipeline: &Pipeline,
        notify: &NotifyConfig,
        message_template: Option<&String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        if let Some(template) = message_template {
            // Build vars for interpolation (namespace pipeline vars under "var." like elsewhere)
            let mut vars: HashMap<String, String> = pipeline
                .vars
                .iter()
                .map(|(k, v)| (format!("var.{}", k), v.clone()))
                .collect();
            vars.insert("pipeline_id".to_string(), pipeline.id.clone());
            vars.insert("name".to_string(), pipeline.name.clone());
            if let Some(err) = &pipeline.error {
                vars.insert("error".to_string(), err.clone());
            }

            let message = NotifyConfig::render(template, &vars);
            let event = self
                .executor
                .execute(Effect::Notify {
                    title: pipeline.name.clone(),
                    message,
                })
                .await?;
            return Ok(event.into_iter().collect());
        }
        let _ = notify; // silence unused warning when no template
        Ok(vec![])
    }

    /// Cancel a running pipeline.
    pub(crate) async fn cancel_pipeline(
        &self,
        pipeline: &Pipeline,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Already terminal — no-op
        if pipeline.is_terminal() {
            tracing::info!(pipeline_id = %pipeline.id, "cancel: pipeline already terminal");
            return Ok(vec![]);
        }

        let effects = steps::cancellation_effects(pipeline);
        let mut result = vec![];
        for effect in effects {
            result.extend(self.executor.execute(effect).await?);
        }

        tracing::info!(pipeline_id = %pipeline.id, "cancelled pipeline");
        Ok(result)
    }
}
