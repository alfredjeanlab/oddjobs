// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared job creation logic used by both command and worker handlers.

use super::super::Runtime;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, JobId, WorkspaceId};
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
            namespace,
            cron_name,
        } = params;

        // Idempotency guard: if job already exists (e.g., from crash recovery
        // where the triggering event is re-processed), skip creation.
        // This prevents workspace creation from failing on the second attempt.
        if self.get_job(job_id.as_str()).is_some() {
            tracing::debug!(
                job_id = %job_id,
                "job already exists, skipping duplicate creation"
            );
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
            oj_runbook::job_display_name(&raw, nonce, &namespace)
        } else {
            job_name
        };

        // Capture notify config before runbook is moved into cache
        let notify_config = job_def.notify.clone();

        // Determine execution path and workspace metadata (path, id, type)
        let is_worktree;
        let workspace_id_str;
        let (execution_path, has_workspace) = match (&job_def.cwd, &job_def.workspace) {
            (Some(cwd), None) => {
                // cwd set, workspace omitted: run directly in cwd (interpolated)
                is_worktree = false;
                workspace_id_str = String::new();
                (PathBuf::from(oj_runbook::interpolate(cwd, &vars)), false)
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

                is_worktree = job_def
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
        let workspace_block = match &job_def.workspace {
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
            vars.insert("workspace.branch".to_string(), branch_name);

            // Ref: interpolate from workspace config, eagerly evaluate $(...) shell expressions
            if let Some(ref template) = workspace_block.as_ref().and_then(|b| b.from_ref.clone()) {
                let value = oj_runbook::interpolate(template, &lookup);
                let value = if value.contains("$(") {
                    let cwd = vars
                        .get("invoke.dir")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                    let mut cmd = tokio::process::Command::new("bash");
                    cmd.arg("-c")
                        .arg(format!("printf '%s' {}", value))
                        .current_dir(&cwd);
                    let output = oj_adapters::subprocess::run_with_timeout(
                        cmd,
                        oj_adapters::subprocess::SHELL_EVAL_TIMEOUT,
                        "evaluate workspace.ref",
                    )
                    .await
                    .map_err(RuntimeError::ShellError)?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(RuntimeError::ShellError(format!(
                            "workspace.ref evaluation failed: {}",
                            stderr.trim()
                        )));
                    }
                    // Strip trailing newlines to match standard $() substitution behavior
                    String::from_utf8_lossy(&output.stdout)
                        .trim_end_matches('\n')
                        .to_string()
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
                    let output = oj_adapters::subprocess::run_with_timeout(
                        cmd,
                        oj_adapters::subprocess::SHELL_EVAL_TIMEOUT,
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
                    String::from_utf8_lossy(&output.stdout)
                        .trim_end_matches('\n')
                        .to_string()
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
                let mut cmd = tokio::process::Command::new("git");
                cmd.args([
                    "-C",
                    &invoke_dir.display().to_string(),
                    "rev-parse",
                    "--show-toplevel",
                ])
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE");
                let repo_root_output = oj_adapters::subprocess::run_with_timeout(
                    cmd,
                    oj_adapters::subprocess::SHELL_EVAL_TIMEOUT,
                    "git rev-parse",
                )
                .await
                .map_err(RuntimeError::ShellError)?;
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
                    .unwrap_or_else(|| format!("ws-{}", job_id.short(8)));
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
                owner: Some(job_id.to_string()),
                workspace_type: workspace_type_str,
                repo_root,
                branch,
                start_point,
            }]
        } else {
            vec![]
        };

        // Compute initial step
        let initial_step = job_def
            .first_step()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "init".to_string());

        // Extract first step info before releasing borrow on runbook
        let first_step_name = job_def.first_step().map(|p| p.name.clone());

        // Phase 1: Persist job record before workspace setup
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
        // Variables with existing scope prefixes (invoke.*, workspace.*, local.*, etc.)
        // are kept as-is. Only bare keys (user input) get the `var.` prefix.
        let namespaced_vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| {
                // Check if key already has a scope prefix
                let has_prefix = k.starts_with("invoke.")
                    || k.starts_with("workspace.")
                    || k.starts_with("local.")
                    || k.starts_with("var.")
                    || k.starts_with("args.")
                    || k.starts_with("item.");
                if has_prefix {
                    (k.clone(), v.clone())
                } else {
                    (format!("var.{}", k), v.clone())
                }
            })
            .collect();

        creation_effects.push(Effect::Emit {
            event: Event::JobCreated {
                id: job_id.clone(),
                kind: job_kind,
                name: job_name.clone(),
                runbook_hash: runbook_hash.clone(),
                cwd: execution_path.clone(),
                vars: namespaced_vars,
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
        self.logger.append(job_id.as_str(), "init", "job created");

        // Write initial breadcrumb after job is persisted
        if let Some(job) = self.get_job(job_id.as_str()) {
            self.breadcrumb.write(&job);
        }

        // Emit on_start notification if configured
        if let Some(template) = &notify_config.on_start {
            let mut notify_vars: HashMap<String, String> = vars
                .iter()
                .map(|(k, v)| (format!("var.{}", k), v.clone()))
                .collect();
            notify_vars.insert("job_id".to_string(), job_id.to_string());
            notify_vars.insert("name".to_string(), job_name.clone());

            let message = NotifyConfig::render(template, &notify_vars);
            if let Some(event) = self
                .executor
                .execute(Effect::Notify {
                    title: job_name.clone(),
                    message,
                })
                .await?
            {
                result_events.push(event);
            }
        }

        // Phase 2: Attempt workspace setup (fails → job marked Failed)
        if !workspace_effects.is_empty() {
            match self.executor.execute_all(workspace_effects).await {
                Ok(ws_events) => result_events.extend(ws_events),
                Err(e) => {
                    let job = self.require_job(job_id.as_str())?;
                    result_events.extend(self.fail_job(&job, &e.to_string()).await?);
                    return Ok(result_events);
                }
            }
        }

        // Start the first step
        if let Some(step_name) = first_step_name {
            result_events.extend(
                self.start_step(&job_id, &step_name, &vars, &execution_path)
                    .await?,
            );
        }

        Ok(result_events)
    }
}
