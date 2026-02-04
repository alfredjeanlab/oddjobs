// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Pipeline lifecycle event handling (resume, cancel, workspace, shell)

use super::super::Runtime;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, PipelineId, SessionId, ShortId, StepOutcome, WorkspaceId};
use std::collections::HashMap;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_pipeline_resume(
        &self,
        pipeline_id: &PipelineId,
        message: Option<&str>,
        vars: &HashMap<String, String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let pipeline = self.require_pipeline(pipeline_id.as_str())?;

        // If pipeline is in terminal "failed" state, find the last failed step
        // from history and reset the pipeline to that step so it can be retried.
        // We track the step name separately because the event may not be applied
        // to state immediately.
        let resume_step = if pipeline.step == "failed" {
            let failed_step = pipeline
                .step_history
                .iter()
                .rev()
                .find(|r| matches!(r.outcome, StepOutcome::Failed(_)))
                .map(|r| r.name.clone())
                .ok_or_else(|| {
                    RuntimeError::InvalidRequest("no failed step found in history".into())
                })?;

            tracing::info!(
                pipeline_id = %pipeline.id,
                failed_step = %failed_step,
                "resuming from terminal failure: resetting to failed step"
            );

            self.executor
                .execute(Effect::Emit {
                    event: Event::PipelineAdvanced {
                        id: pipeline_id.clone(),
                        step: failed_step.clone(),
                    },
                })
                .await?;

            failed_step
        } else {
            pipeline.step.clone()
        };

        // Persist var updates if any
        if !vars.is_empty() {
            self.executor
                .execute(Effect::Emit {
                    event: Event::PipelineUpdated {
                        id: PipelineId::new(&pipeline.id),
                        vars: vars.clone(),
                    },
                })
                .await?;
        }

        // Merge vars for this resume operation
        let merged_inputs: HashMap<String, String> = pipeline
            .vars
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .chain(vars.clone())
            .collect();

        // Determine step type from runbook
        let runbook = self.cached_runbook(&pipeline.runbook_hash)?;
        let pipeline_def = runbook
            .get_pipeline(&pipeline.kind)
            .ok_or_else(|| RuntimeError::PipelineDefNotFound(pipeline.kind.clone()))?;
        let step_def = pipeline_def
            .get_step(&resume_step)
            .ok_or_else(|| RuntimeError::StepNotFound(resume_step.clone()))?;

        if step_def.is_agent() {
            // Agent step: require message
            let msg = message.ok_or_else(|| {
                RuntimeError::InvalidRequest(format!(
                    "agent steps require --message for resume. Example:\n  \
                     oj pipeline resume {} -m \"I fixed the import, try again\"",
                    pipeline.id.short(12)
                ))
            })?;

            let agent_name = step_def
                .agent_name()
                .ok_or_else(|| RuntimeError::AgentNotFound("no agent name in step".into()))?;

            self.handle_agent_resume(&pipeline, &resume_step, agent_name, msg, &merged_inputs)
                .await
        } else if step_def.is_shell() {
            // Shell step: re-run command
            if message.is_some() {
                tracing::warn!(
                    pipeline_id = %pipeline.id,
                    "resume --message ignored for shell steps"
                );
            }

            let command = step_def
                .shell_command()
                .ok_or_else(|| RuntimeError::InvalidRequest("no shell command in step".into()))?;

            self.handle_shell_resume(&pipeline, &resume_step, command)
                .await
        } else {
            Err(RuntimeError::InvalidRequest(format!(
                "resume not supported for step type in step: {}",
                resume_step
            )))
        }
    }

    pub(crate) async fn handle_pipeline_cancel(
        &self,
        pipeline_id: &PipelineId,
    ) -> Result<Vec<Event>, RuntimeError> {
        let pipeline = self
            .get_pipeline(pipeline_id.as_str())
            .ok_or_else(|| RuntimeError::PipelineNotFound(pipeline_id.to_string()))?;
        self.cancel_pipeline(&pipeline).await
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
        pipeline: &oj_core::Pipeline,
        step: &str,
        _command: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Kill existing session if any
        if let Some(session_id) = &pipeline.session_id {
            let _ = self
                .executor
                .execute(Effect::KillSession {
                    session_id: SessionId::new(session_id),
                })
                .await;
        }

        // Re-run the shell command
        let execution_dir = self.execution_dir(pipeline);
        let pipeline_id = PipelineId::new(&pipeline.id);
        let result = self
            .start_step(&pipeline_id, step, &pipeline.vars, &execution_dir)
            .await?;

        tracing::info!(pipeline_id = %pipeline.id, "re-running shell step");
        Ok(result)
    }

    pub(crate) async fn handle_shell_exited(
        &self,
        pipeline_id: &PipelineId,
        step: &str,
        exit_code: i32,
        stdout: Option<&str>,
        stderr: Option<&str>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let pipeline = self.require_pipeline(pipeline_id.as_str())?;

        // Verify we're in the expected step
        if pipeline.step != step {
            tracing::warn!(
                pipeline_id = %pipeline_id,
                expected = step,
                actual = %pipeline.step,
                "shell completed for unexpected step"
            );
            return Ok(vec![]);
        }

        // Write captured output before the status line
        if let Some(out) = stdout {
            self.logger
                .append_fenced(pipeline_id.as_str(), step, "stdout", out);
        }
        if let Some(err) = stderr {
            self.logger
                .append_fenced(pipeline_id.as_str(), step, "stderr", err);
        }

        if exit_code == 0 {
            self.logger.append(
                pipeline_id.as_str(),
                step,
                &format!("shell completed (exit {})", exit_code),
            );
            self.advance_pipeline(&pipeline).await
        } else {
            self.logger.append(
                pipeline_id.as_str(),
                step,
                &format!("shell failed (exit {})", exit_code),
            );
            self.fail_pipeline(&pipeline, &format!("shell exit code: {}", exit_code))
                .await
        }
    }
}
