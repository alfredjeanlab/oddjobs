// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command run event handling

use super::super::Runtime;
use super::CreatePipelineParams;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, PipelineId};
use oj_runbook::RunDirective;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

impl<S, A, C> Runtime<S, A, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    C: Clock,
{
    // TODO(refactor): group command handler parameters into a struct
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_command(
        &self,
        pipeline_id: &PipelineId,
        pipeline_name: &str,
        project_root: &Path,
        invoke_dir: &Path,
        namespace: &str,
        command: &str,
        args: &HashMap<String, String>,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Load runbook from project
        let runbook = self.load_runbook_for_command(project_root, command)?;

        // Serialize and hash the runbook for WAL storage
        let runbook_json = serde_json::to_value(&runbook).map_err(|e| {
            RuntimeError::RunbookLoadError(format!("failed to serialize runbook: {}", e))
        })?;
        let runbook_hash = {
            let canonical = serde_json::to_string(&runbook_json).map_err(|e| {
                RuntimeError::RunbookLoadError(format!("failed to serialize runbook: {}", e))
            })?;
            let digest = Sha256::digest(canonical.as_bytes());
            format!("{:x}", digest)
        };

        // Inject invoke.dir so runbooks can reference ${invoke.dir}
        let mut args = args.clone();
        args.entry("invoke.dir".to_string())
            .or_insert_with(|| invoke_dir.display().to_string());

        let cmd_def = runbook
            .get_command(command)
            .ok_or_else(|| RuntimeError::CommandNotFound(command.to_string()))?;

        match &cmd_def.run {
            RunDirective::Pipeline { .. } => {
                // Validate pipeline def exists
                let _ = runbook
                    .get_pipeline(pipeline_name)
                    .ok_or_else(|| RuntimeError::PipelineDefNotFound(pipeline_name.to_string()))?;

                let name = args
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| pipeline_id.to_string());

                // Only pass runbook_json if not already cached
                let already_cached = self.runbook_cache.lock().contains_key(&runbook_hash);
                let runbook_json_param = if already_cached {
                    None
                } else {
                    Some(runbook_json)
                };

                self.create_and_start_pipeline(CreatePipelineParams {
                    pipeline_id: pipeline_id.clone(),
                    pipeline_name: name,
                    pipeline_kind: pipeline_name.to_string(),
                    vars: args.clone(),
                    runbook_hash,
                    runbook_json: runbook_json_param,
                    runbook,
                    namespace: namespace.to_string(),
                })
                .await
            }
            RunDirective::Shell(cmd) => {
                let cmd = cmd.clone();
                let name = args
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| pipeline_id.to_string());
                let step_name = "run";
                let execution_path = project_root.to_path_buf();

                // Phase 1: Persist pipeline record
                let mut creation_effects = Vec::new();
                let already_cached = self.runbook_cache.lock().contains_key(&runbook_hash);
                if !already_cached {
                    creation_effects.push(Effect::Emit {
                        event: Event::RunbookLoaded {
                            hash: runbook_hash.clone(),
                            version: 1,
                            runbook: runbook_json,
                        },
                    });
                }

                creation_effects.push(Effect::Emit {
                    event: Event::PipelineCreated {
                        id: pipeline_id.clone(),
                        kind: command.to_string(),
                        name: name.clone(),
                        runbook_hash: runbook_hash.clone(),
                        cwd: execution_path.clone(),
                        vars: args.clone(),
                        initial_step: step_name.to_string(),
                        created_at_epoch_ms: self.clock().epoch_ms(),
                        namespace: namespace.to_string(),
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
                    .append(pipeline_id.as_str(), step_name, "shell command created");

                // Phase 2: Interpolate and execute shell command
                // Values are escaped by interpolate_shell() during substitution
                let mut vars: HashMap<String, String> = args
                    .iter()
                    .map(|(k, v)| (format!("args.{}", k), v.clone()))
                    .collect();
                vars.insert("pipeline_id".to_string(), pipeline_id.to_string());
                vars.insert("name".to_string(), name.clone());
                vars.insert(
                    "workspace".to_string(),
                    execution_path.display().to_string(),
                );

                let interpolated = oj_runbook::interpolate_shell(&cmd, &vars);
                self.logger.append(
                    pipeline_id.as_str(),
                    step_name,
                    &format!(
                        "shell (cwd: {}): {}",
                        execution_path.display(),
                        interpolated
                    ),
                );

                let shell_effects = vec![
                    Effect::Emit {
                        event: Event::StepStarted {
                            pipeline_id: pipeline_id.clone(),
                            step: step_name.to_string(),
                            agent_id: None,
                        },
                    },
                    Effect::Shell {
                        pipeline_id: pipeline_id.clone(),
                        step: step_name.to_string(),
                        command: interpolated,
                        cwd: execution_path,
                        env: if namespace.is_empty() {
                            HashMap::new()
                        } else {
                            HashMap::from([("OJ_NAMESPACE".to_string(), namespace.to_string())])
                        },
                    },
                ];
                result_events.extend(self.executor.execute_all(shell_effects).await?);

                Ok(result_events)
            }
            RunDirective::Agent { agent } => {
                Err(Self::invalid_directive("command", "agent", agent))
            }
        }
    }
}
