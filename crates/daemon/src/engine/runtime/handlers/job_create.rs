// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared job creation logic used by both command and worker handlers.

use super::super::Runtime;
use crate::engine::error::RuntimeError;
use oj_core::{Clock, Effect, Event, JobId};
use oj_runbook::{NotifyConfig, Runbook};
use std::collections::HashMap;
use std::path::PathBuf;

/// Parameters for creating and starting a job
pub(crate) struct CreateJobParams {
    pub job_id: JobId,
    pub job_name: String,
    pub job_kind: String,
    pub vars: HashMap<String, String>,
    pub runbook_hash: String,
    pub runbook_json: Option<serde_json::Value>,
    pub runbook: Runbook,
    pub project: String,
    pub cron_name: Option<String>,
}

impl<C: Clock> Runtime<C> {
    pub(crate) async fn create_and_start_job(
        &self,
        params: CreateJobParams,
    ) -> Result<Vec<Event>, RuntimeError> {
        let CreateJobParams {
            job_id,
            job_name,
            job_kind,
            mut vars,
            runbook_hash,
            runbook_json,
            runbook,
            project,
            cron_name,
        } = params;

        // Idempotency guard: if job already exists (e.g., from crash recovery
        // where the triggering event is re-processed), skip creation.
        // This prevents workspace creation from failing on the second attempt.
        if self.get_job(job_id.as_str()).is_some() {
            return Ok(vec![]);
        }

        // Look up job definition
        let job_def = runbook
            .get_job(&job_kind)
            .ok_or_else(|| RuntimeError::JobDefNotFound(job_kind.clone()))?;

        // Resolve job display name from template (if set)
        let job_name = if let Some(name_template) = &job_def.name {
            let nonce = job_id.short(8);
            let lookup: HashMap<String, String> = vars
                .iter()
                .flat_map(|(k, v)| vec![(k.clone(), v.clone()), (format!("var.{}", k), v.clone())])
                .collect();
            let raw = oj_runbook::interpolate(name_template, &lookup);
            oj_runbook::job_display_name(&raw, nonce, &project)
        } else {
            job_name
        };

        // Capture notify config before runbook is moved into cache
        let notify_config = job_def.notify.clone();

        // Determine execution path and workspace metadata (path, id, type)
        let is_worktree;
        let execution_path = match (&job_def.cwd, &job_def.source) {
            (Some(cwd), None) => {
                // cwd set, workspace omitted: run directly in cwd (interpolated)
                is_worktree = false;
                PathBuf::from(oj_runbook::interpolate(cwd, &vars))
            }
            (Some(_), Some(_)) | (None, Some(_)) => {
                // Create workspace directory
                let nonce = job_id.short(8);
                let ws_name = job_name.strip_prefix("oj-").unwrap_or(&job_name);
                let ws_id = if ws_name.ends_with(nonce) {
                    format!("ws-{}", ws_name)
                } else {
                    format!("ws-{}-{}", ws_name, nonce)
                };

                // Compute workspace path from state_dir
                let workspaces_dir = self.state_dir.join("workspaces");
                let workspace_path = workspaces_dir.join(&ws_id);

                is_worktree = job_def.source.as_ref().map(|w| w.is_git_worktree()).unwrap_or(false);

                // Inject source template variables
                let ws_root = workspace_path.display().to_string();
                let ws_type = if is_worktree { "worktree" } else { "folder" }.to_string();
                vars.insert("source.id".to_string(), ws_id);
                vars.insert("source.root".to_string(), ws_root);
                vars.insert("source.nonce".to_string(), nonce.to_string());
                vars.insert("source.type".to_string(), ws_type);

                workspace_path
            }
            // Default: run in cwd (where oj CLI was invoked)
            (None, None) => {
                is_worktree = false;
                vars.get("invoke.dir")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
            }
        };

        // Interpolate source.branch and source.ref from source config
        // (before locals, so locals can reference ${source.branch} if needed)
        let workspace_block = match &job_def.source {
            Some(oj_runbook::WorkspaceConfig::Block(block)) => Some(block.clone()),
            _ => None,
        };

        if is_worktree {
            let nonce = job_id.short(8);

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
            vars.insert("source.branch".to_string(), branch_name);

            // Ref: interpolate from workspace config, eagerly evaluate $(...) shell expressions
            if let Some(ref template) = workspace_block.as_ref().and_then(|b| b.from_ref.clone()) {
                let value = oj_runbook::interpolate(template, &lookup);
                let value = if value.contains("$(") {
                    let cwd = vars
                        .get("invoke.dir")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                    let mut cmd = tokio::process::Command::new("bash");
                    cmd.arg("-c").arg(format!("printf '%s' {}", value)).current_dir(&cwd);
                    let output = crate::adapters::subprocess::run_with_timeout(
                        cmd,
                        crate::adapters::subprocess::SHELL_EVAL_TIMEOUT,
                        "evaluate source.ref",
                    )
                    .await
                    .map_err(RuntimeError::ShellError)?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(RuntimeError::ShellError(format!(
                            "source.ref evaluation failed: {}",
                            stderr.trim()
                        )));
                    }
                    // Strip trailing newlines to match standard $() substitution behavior
                    String::from_utf8_lossy(&output.stdout).trim_end_matches('\n').to_string()
                } else {
                    value
                };
                vars.insert("source.ref".to_string(), value);
            }

            // Resolve repo root now so it's persisted in vars for handle_job_created
            let invoke_dir = vars
                .get("invoke.dir")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let mut cmd = tokio::process::Command::new("git");
            cmd.args(["-C", &invoke_dir.display().to_string(), "rev-parse", "--show-toplevel"])
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE");
            let repo_root_output = crate::adapters::subprocess::run_with_timeout(
                cmd,
                crate::adapters::subprocess::SHELL_EVAL_TIMEOUT,
                "git rev-parse",
            )
            .await
            .map_err(RuntimeError::ShellError)?;
            if !repo_root_output.status.success() {
                return Err(RuntimeError::ShellError(
                    "git rev-parse --show-toplevel failed: not a git repository".to_string(),
                ));
            }
            let repo_root = String::from_utf8_lossy(&repo_root_output.stdout).trim().to_string();
            vars.insert("source.repo_root".to_string(), repo_root);

            // Resolve git remote URL so container adapters don't need a local checkout
            let mut cmd = tokio::process::Command::new("git");
            cmd.args(["remote", "get-url", "origin"])
                .current_dir(&invoke_dir)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE");
            if let Ok(output) = crate::adapters::subprocess::run_with_timeout(
                cmd,
                crate::adapters::subprocess::SHELL_EVAL_TIMEOUT,
                "git remote get-url",
            )
            .await
            {
                if output.status.success() {
                    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !url.is_empty() {
                        vars.insert("source.repo".to_string(), url);
                    }
                }
            }
        }

        // Evaluate locals: interpolate each value with current vars, then add as local.*
        // Build a lookup map that includes var.*-prefixed keys so templates like
        // ${var.name} resolve (the vars map stores raw keys like "name").
        // Shell expressions $(...) are eagerly evaluated so locals become plain data.
        if !job_def.locals.is_empty() {
            let mut lookup: HashMap<String, String> = vars
                .iter()
                .flat_map(|(k, v)| {
                    let prefixed = format!("var.{}", k);
                    vec![(k.clone(), v.clone()), (prefixed, v.clone())]
                })
                .collect();
            for (key, template) in &job_def.locals {
                let has_shell = template.contains("$(");
                let value = if has_shell {
                    oj_runbook::interpolate_shell(template, &lookup)
                } else {
                    oj_runbook::interpolate(template, &lookup)
                };

                // Eagerly evaluate shell expressions — $(cmd) becomes plain data
                let value = if has_shell {
                    let cwd = vars
                        .get("invoke.dir")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                    let trimmed = value.trim();
                    // Strip $(...) wrapper and run inner command directly to avoid
                    // word-splitting. For mixed literal+shell, use printf wrapper.
                    let shell_cmd = if trimmed.starts_with("$(") && trimmed.ends_with(')') {
                        trimmed[2..trimmed.len() - 1].to_string()
                    } else {
                        format!("printf '%s' \"{}\"", value)
                    };
                    let desc = format!("evaluate local.{}", key);
                    let mut cmd = tokio::process::Command::new("bash");
                    cmd.arg("-c").arg(&shell_cmd).current_dir(&cwd);
                    let output = crate::adapters::subprocess::run_with_timeout(
                        cmd,
                        crate::adapters::subprocess::SHELL_EVAL_TIMEOUT,
                        &desc,
                    )
                    .await
                    .map_err(RuntimeError::ShellError)?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(RuntimeError::ShellError(format!(
                            "local.{} evaluation failed: {}",
                            key,
                            stderr.trim()
                        )));
                    }
                    // Strip trailing newlines to match standard $() substitution behavior
                    String::from_utf8_lossy(&output.stdout).trim_end_matches('\n').to_string()
                } else {
                    value
                };

                lookup.insert(format!("local.{}", key), value.clone());
                vars.insert(format!("local.{}", key), value);
            }
        }

        // Compute initial step
        let initial_step =
            job_def.first_step().map(|p| p.name.clone()).unwrap_or_else(|| "init".to_string());

        // Persist job record — workspace creation and step start are handled
        // asynchronously by handle_job_created() when the JobCreated event
        // is processed by the event loop.
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

        // Namespace user input variables with `var.` prefix for display isolation.
        let namespaced_vars = crate::engine::vars::namespace_vars(&vars);

        creation_effects.push(Effect::Emit {
            event: Event::JobCreated {
                id: job_id,
                kind: job_kind,
                name: job_name.clone(),
                runbook_hash: runbook_hash.clone(),
                cwd: execution_path.clone(),
                vars: namespaced_vars,
                initial_step: initial_step.clone(),
                created_at_ms: self.executor.clock().epoch_ms(),
                project: project.clone(),
                cron: cron_name,
            },
        });

        // Insert into in-process cache
        {
            self.runbook_cache.lock().entry(runbook_hash).or_insert(runbook);
        }

        let mut result_events = self.executor.execute_all(creation_effects).await?;
        self.logger.append(job_id.as_str(), "init", "job created");

        // Write initial breadcrumb after job is persisted
        if let Some(job) = self.get_job(job_id.as_str()) {
            self.breadcrumb.write(&job);
        }

        // Emit on_start notification if configured
        if let Some(template) = &notify_config.on_start {
            let mut notify_vars = crate::engine::vars::namespace_vars(&vars);
            notify_vars.insert("job_id".to_string(), job_id.to_string());
            notify_vars.insert("name".to_string(), job_name.clone());

            let message = NotifyConfig::render(template, &notify_vars);
            if let Some(event) =
                self.executor.execute(Effect::Notify { title: job_name.clone(), message }).await?
            {
                result_events.push(event);
            }
        }

        Ok(result_events)
    }
}
