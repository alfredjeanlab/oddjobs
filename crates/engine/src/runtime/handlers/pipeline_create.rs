// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared pipeline creation logic used by both command and worker handlers.

use super::super::Runtime;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, PipelineId, WorkspaceId};
use oj_runbook::{NotifyConfig, Runbook};
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
    pub cron_name: Option<String>,
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
            cron_name,
        } = params;

        // Look up pipeline definition
        let pipeline_def = runbook
            .get_pipeline(&pipeline_kind)
            .ok_or_else(|| RuntimeError::PipelineDefNotFound(pipeline_kind.clone()))?;

        // Resolve pipeline display name from template (if set)
        let pipeline_name = if let Some(name_template) = &pipeline_def.name {
            let nonce = pipeline_id.short(8);
            let lookup: HashMap<String, String> = vars
                .iter()
                .flat_map(|(k, v)| vec![(k.clone(), v.clone()), (format!("var.{}", k), v.clone())])
                .collect();
            let raw = oj_runbook::interpolate(name_template, &lookup);
            oj_runbook::pipeline_display_name(&raw, nonce, &namespace)
        } else {
            pipeline_name
        };

        // Capture notify config before runbook is moved into cache
        let notify_config = pipeline_def.notify.clone();

        // Determine execution path and workspace metadata (path, id, type)
        let is_worktree;
        let workspace_id_str;
        let (execution_path, has_workspace) = match (&pipeline_def.cwd, &pipeline_def.workspace) {
            (Some(cwd), None) => {
                // cwd set, workspace omitted: run directly in cwd (interpolated)
                is_worktree = false;
                workspace_id_str = String::new();
                (PathBuf::from(oj_runbook::interpolate(cwd, &vars)), false)
            }
            (Some(_), Some(_)) | (None, Some(_)) => {
                // Create workspace directory
                let nonce = pipeline_id.short(8);
                let ws_name = pipeline_name.strip_prefix("oj-").unwrap_or(&pipeline_name);
                let ws_id = if ws_name.ends_with(nonce) {
                    format!("ws-{}", ws_name)
                } else {
                    format!("ws-{}-{}", ws_name, nonce)
                };

                // Compute workspace path from state_dir
                let workspaces_dir = self.state_dir.join("workspaces");
                let workspace_path = workspaces_dir.join(&ws_id);

                is_worktree = pipeline_def
                    .workspace
                    .as_ref()
                    .map(|w| w.is_git_worktree())
                    .unwrap_or(false);

                // Inject workspace template variables
                vars.insert("workspace.id".to_string(), ws_id.clone());
                vars.insert(
                    "workspace.root".to_string(),
                    workspace_path.display().to_string(),
                );
                vars.insert("workspace.nonce".to_string(), nonce.to_string());

                workspace_id_str = ws_id;
                (workspace_path, true)
            }
            // Default: run in cwd (where oj CLI was invoked)
            (None, None) => {
                is_worktree = false;
                workspace_id_str = String::new();
                let cwd = vars
                    .get("invoke.dir")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                (cwd, false)
            }
        };

        // Interpolate workspace.branch and workspace.ref from workspace config
        // (before locals, so locals can reference ${workspace.branch} if needed)
        let workspace_block = match &pipeline_def.workspace {
            Some(oj_runbook::WorkspaceConfig::Block(block)) => Some(block.clone()),
            _ => None,
        };

        if is_worktree {
            let nonce = pipeline_id.short(8);

            // Build lookup for interpolation (same pattern as locals)
            let lookup: HashMap<String, String> = vars
                .iter()
                .flat_map(|(k, v)| {
                    let prefixed = format!("var.{}", k);
                    vec![(k.clone(), v.clone()), (prefixed, v.clone())]
                })
                .collect();

            // Branch: interpolate from workspace config, or auto-generate ws-<nonce>
            let branch_name = if let Some(ref template) =
                workspace_block.as_ref().and_then(|b| b.branch.clone())
            {
                oj_runbook::interpolate(template, &lookup)
            } else {
                format!("ws-{}", nonce)
            };
            vars.insert("workspace.branch".to_string(), branch_name);

            // Ref: interpolate from workspace config, eagerly evaluate $(...) shell expressions
            if let Some(ref template) = workspace_block.as_ref().and_then(|b| b.from_ref.clone()) {
                let value = oj_runbook::interpolate(template, &lookup);
                let value = if value.contains("$(") {
                    let cwd = vars
                        .get("invoke.dir")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                    let output = tokio::process::Command::new("bash")
                        .arg("-c")
                        .arg(format!("printf '%s' {}", value))
                        .current_dir(&cwd)
                        .output()
                        .await
                        .map_err(|e| {
                            RuntimeError::ShellError(format!(
                                "failed to evaluate workspace.ref: {}",
                                e
                            ))
                        })?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(RuntimeError::ShellError(format!(
                            "workspace.ref evaluation failed: {}",
                            stderr.trim()
                        )));
                    }
                    String::from_utf8_lossy(&output.stdout).to_string()
                } else {
                    value
                };
                vars.insert("workspace.ref".to_string(), value);
            }
        }

        // Evaluate locals: interpolate each value with current vars, then add as local.*
        // Build a lookup map that includes var.*-prefixed keys so templates like
        // ${var.name} resolve (the vars map stores raw keys like "name").
        // Shell expressions $(...) are eagerly evaluated so locals become plain data.
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

                // Eagerly evaluate shell expressions — $(cmd) becomes plain data
                let value = if value.contains("$(") {
                    let cwd = vars
                        .get("invoke.dir")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                    let output = tokio::process::Command::new("bash")
                        .arg("-c")
                        .arg(format!("printf '%s' {}", value))
                        .current_dir(&cwd)
                        .output()
                        .await
                        .map_err(|e| {
                            RuntimeError::ShellError(format!(
                                "failed to evaluate local.{}: {}",
                                key, e
                            ))
                        })?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(RuntimeError::ShellError(format!(
                            "local.{} evaluation failed: {}",
                            key,
                            stderr.trim()
                        )));
                    }
                    String::from_utf8_lossy(&output.stdout).to_string()
                } else {
                    value
                };

                lookup.insert(format!("local.{}", key), value.clone());
                vars.insert(format!("local.{}", key), value);
            }
        }

        // Build workspace effects using workspace.branch and workspace.ref from vars
        let workspace_effects = if has_workspace {
            let (repo_root, branch, start_point) = if is_worktree {
                let invoke_dir = vars
                    .get("invoke.dir")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                let repo_root_output = tokio::process::Command::new("git")
                    .args([
                        "-C",
                        &invoke_dir.display().to_string(),
                        "rev-parse",
                        "--show-toplevel",
                    ])
                    .env_remove("GIT_DIR")
                    .env_remove("GIT_WORK_TREE")
                    .output()
                    .await
                    .map_err(|e| {
                        RuntimeError::ShellError(format!("git rev-parse failed: {}", e))
                    })?;
                if !repo_root_output.status.success() {
                    return Err(RuntimeError::ShellError(
                        "git rev-parse --show-toplevel failed: not a git repository".to_string(),
                    ));
                }
                let repo_root = PathBuf::from(
                    String::from_utf8_lossy(&repo_root_output.stdout)
                        .trim()
                        .to_string(),
                );

                // Safety: workspace.branch is always injected above when is_worktree
                let branch_name = vars
                    .get("workspace.branch")
                    .cloned()
                    .unwrap_or_else(|| format!("ws-{}", pipeline_id.short(8)));
                let start_point = vars
                    .get("workspace.ref")
                    .cloned()
                    .unwrap_or_else(|| "HEAD".to_string());

                (Some(repo_root), Some(branch_name), Some(start_point))
            } else {
                (None, None, None)
            };

            let workspace_type_str = if is_worktree {
                Some("worktree".to_string())
            } else {
                Some("folder".to_string())
            };

            vec![Effect::CreateWorkspace {
                workspace_id: WorkspaceId::new(workspace_id_str),
                path: execution_path.clone(),
                owner: Some(pipeline_id.to_string()),
                workspace_type: workspace_type_str,
                repo_root,
                branch,
                start_point,
            }]
        } else {
            vec![]
        };

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
                cron_name,
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

        // Write initial breadcrumb after pipeline is persisted
        if let Some(pipeline) = self.get_pipeline(pipeline_id.as_str()) {
            self.breadcrumb.write(&pipeline);
        }

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
