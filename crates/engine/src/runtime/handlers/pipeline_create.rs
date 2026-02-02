// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared pipeline creation logic used by both command and worker handlers.

use super::super::Runtime;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, PipelineId, WorkspaceId};
use oj_runbook::{NotifyConfig, Runbook, WorkspaceMode};
use std::collections::HashMap;
use std::path::PathBuf;

/// Parameters for creating and starting a pipeline
pub(crate) struct CreatePipelineParams {
    pub pipeline_id: PipelineId,
    pub pipeline_name: String,
    pub pipeline_kind: String,
    pub vars: HashMap<String, String>,
    pub runbook_hash: String,
    pub runbook_json: Option<serde_json::Value>,
    pub runbook: Runbook,
    pub namespace: String,
}

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn create_and_start_pipeline(
        &self,
        params: CreatePipelineParams,
    ) -> Result<Vec<Event>, RuntimeError> {
        let CreatePipelineParams {
            pipeline_id,
            pipeline_name,
            pipeline_kind,
            mut vars,
            runbook_hash,
            runbook_json,
            runbook,
            namespace,
        } = params;

        // Look up pipeline definition
        let pipeline_def = runbook
            .get_pipeline(&pipeline_kind)
            .ok_or_else(|| RuntimeError::PipelineDefNotFound(pipeline_kind.clone()))?;

        // Capture notify config before runbook is moved into cache
        let notify_config = pipeline_def.notify.clone();

        // Determine execution path: cwd-only runs in-place, workspace creates directory
        let (execution_path, workspace_effects) = match (&pipeline_def.cwd, &pipeline_def.workspace)
        {
            (Some(cwd), None) => {
                // cwd set, workspace omitted: run directly in cwd (interpolated)
                (PathBuf::from(oj_runbook::interpolate(cwd, &vars)), vec![])
            }
            (Some(_), Some(_)) | (None, Some(_)) => {
                // Create workspace directory
                let pipeline_id_str = pipeline_id.as_str();
                let nonce = &pipeline_id_str[..8.min(pipeline_id_str.len())];
                let workspace_id = format!("ws-{}-{}", pipeline_name, nonce);

                // Compute workspace path from OJ_STATE_DIR
                let state_dir = std::env::var("OJ_STATE_DIR").unwrap_or_else(|_| {
                    format!(
                        "{}/.local/state/oj",
                        std::env::var("HOME").unwrap_or_default()
                    )
                });
                let workspaces_dir = PathBuf::from(&state_dir).join("workspaces");
                let workspace_path = workspaces_dir.join(&workspace_id);

                let mode_str = pipeline_def.workspace.as_ref().map(|m| match m {
                    WorkspaceMode::Ephemeral => "ephemeral".to_string(),
                    WorkspaceMode::Persistent => "persistent".to_string(),
                });

                // Inject workspace template variables
                vars.insert("workspace.id".to_string(), workspace_id.clone());
                vars.insert(
                    "workspace.root".to_string(),
                    workspace_path.display().to_string(),
                );
                vars.insert("workspace.nonce".to_string(), nonce.to_string());

                (
                    workspace_path.clone(),
                    vec![Effect::CreateWorkspace {
                        workspace_id: WorkspaceId::new(workspace_id),
                        path: workspace_path,
                        owner: Some(pipeline_id.to_string()),
                        mode: mode_str,
                    }],
                )
            }
            // Default: run in cwd (where oj CLI was invoked)
            (None, None) => {
                let cwd = vars
                    .get("invoke.dir")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                (cwd, vec![])
            }
        };

        // Evaluate locals: interpolate each value with current vars, then add as local.*
        // Build a lookup map that includes var.*-prefixed keys so templates like
        // ${var.name} resolve (the vars map stores raw keys like "name").
        if !pipeline_def.locals.is_empty() {
            let mut lookup: HashMap<String, String> = vars
                .iter()
                .flat_map(|(k, v)| {
                    let prefixed = format!("var.{}", k);
                    vec![(k.clone(), v.clone()), (prefixed, v.clone())]
                })
                .collect();
            for (key, template) in &pipeline_def.locals {
                let value = oj_runbook::interpolate(template, &lookup);
                lookup.insert(format!("local.{}", key), value.clone());
                vars.insert(format!("local.{}", key), value);
            }
        }

        // Compute initial step
        let initial_step = pipeline_def
            .first_step()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "init".to_string());

        // Extract first step info before releasing borrow on runbook
        let first_step_name = pipeline_def.first_step().map(|p| p.name.clone());

        // Phase 1: Persist pipeline record before workspace setup
        let mut creation_effects = Vec::new();
        if let Some(json) = runbook_json {
            creation_effects.push(Effect::Emit {
                event: Event::RunbookLoaded {
                    hash: runbook_hash.clone(),
                    version: 1,
                    runbook: json,
                },
            });
        }

        creation_effects.push(Effect::Emit {
            event: Event::PipelineCreated {
                id: pipeline_id.clone(),
                kind: pipeline_kind,
                name: pipeline_name.clone(),
                runbook_hash: runbook_hash.clone(),
                cwd: execution_path.clone(),
                vars: vars.clone(),
                initial_step: initial_step.clone(),
                created_at_epoch_ms: self.clock().epoch_ms(),
                namespace: namespace.clone(),
            },
        });

        // Insert into in-process cache
        {
            self.runbook_cache
                .lock()
                .entry(runbook_hash)
                .or_insert(runbook);
        }

        let mut result_events = self.executor.execute_all(creation_effects).await?;
        self.logger
            .append(pipeline_id.as_str(), "init", "pipeline created");

        // Emit on_start notification if configured
        if let Some(template) = &notify_config.on_start {
            let mut notify_vars: HashMap<String, String> = vars
                .iter()
                .map(|(k, v)| (format!("var.{}", k), v.clone()))
                .collect();
            notify_vars.insert("pipeline_id".to_string(), pipeline_id.to_string());
            notify_vars.insert("name".to_string(), pipeline_name.clone());

            let message = NotifyConfig::render(template, &notify_vars);
            if let Some(event) = self
                .executor
                .execute(Effect::Notify {
                    title: pipeline_name.clone(),
                    message,
                })
                .await?
            {
                result_events.push(event);
            }
        }

        // Phase 2: Attempt workspace setup (fails → pipeline marked Failed)
        if !workspace_effects.is_empty() {
            match self.executor.execute_all(workspace_effects).await {
                Ok(ws_events) => result_events.extend(ws_events),
                Err(e) => {
                    let pipeline = self.require_pipeline(pipeline_id.as_str())?;
                    result_events.extend(self.fail_pipeline(&pipeline, &e.to_string()).await?);
                    return Ok(result_events);
                }
            }
        }

        // Start the first step
        if let Some(step_name) = first_step_name {
            result_events.extend(
                self.start_step(&pipeline_id, &step_name, &vars, &execution_path)
                    .await?,
            );
        }

        Ok(result_events)
    }
}
