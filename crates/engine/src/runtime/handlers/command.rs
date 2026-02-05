// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command run event handling

use super::super::Runtime;
use super::CreateJobParams;
use crate::error::RuntimeError;
use crate::runtime::agent_run::SpawnAgentParams;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{AgentRunId, Clock, Effect, Event, JobId};
use oj_runbook::RunDirective;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

/// Parameters for handling a command run event.
pub(crate) struct HandleCommandParams<'a> {
    pub job_id: &'a JobId,
    pub job_name: &'a str,
    pub project_root: &'a Path,
    pub invoke_dir: &'a Path,
    pub namespace: &'a str,
    pub command: &'a str,
    pub args: &'a HashMap<String, String>,
}

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_command(
        &self,
        params: HandleCommandParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let HandleCommandParams {
            job_id,
            job_name,
            project_root,
            invoke_dir,
            namespace,
            command,
            args,
        } = params;

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
            RunDirective::Job { .. } => {
                // Validate job def exists
                let _ = runbook
                    .get_job(job_name)
                    .ok_or_else(|| RuntimeError::JobDefNotFound(job_name.to_string()))?;

                let name = args
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| job_id.to_string());

                // Only pass runbook_json if not already cached
                let already_cached = self.runbook_cache.lock().contains_key(&runbook_hash);
                let runbook_json_param = if already_cached {
                    None
                } else {
                    Some(runbook_json)
                };

                self.create_and_start_job(CreateJobParams {
                    job_id: job_id.clone(),
                    job_name: name,
                    job_kind: job_name.to_string(),
                    vars: args.clone(),
                    runbook_hash,
                    runbook_json: runbook_json_param,
                    runbook,
                    namespace: namespace.to_string(),
                    cron_name: None,
                })
                .await
            }
            RunDirective::Shell(cmd) => {
                // Idempotency guard: if job already exists (e.g., from crash recovery
                // where the CommandRun event is re-processed), skip creation.
                if self.get_job(job_id.as_str()).is_some() {
                    tracing::debug!(
                        job_id = %job_id,
                        "job already exists, skipping duplicate shell command creation"
                    );
                    return Ok(vec![]);
                }

                let cmd = cmd.clone();
                let name = args
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| job_id.to_string());
                let step_name = "run";
                let execution_path = project_root.to_path_buf();

                // Phase 1: Persist job record
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
                    event: Event::JobCreated {
                        id: job_id.clone(),
                        kind: command.to_string(),
                        name: name.clone(),
                        runbook_hash: runbook_hash.clone(),
                        cwd: execution_path.clone(),
                        vars: args.clone(),
                        initial_step: step_name.to_string(),
                        created_at_epoch_ms: self.clock().epoch_ms(),
                        namespace: namespace.to_string(),
                        cron_name: None,
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
                    .append(job_id.as_str(), step_name, "shell command created");

                // Phase 2: Interpolate and execute shell command
                // Values are escaped by interpolate_shell() during substitution
                let mut vars: HashMap<String, String> = args
                    .iter()
                    .map(|(k, v)| (format!("args.{}", k), v.clone()))
                    .collect();
                vars.insert("job_id".to_string(), job_id.to_string());
                vars.insert("name".to_string(), name.clone());
                vars.insert(
                    "workspace".to_string(),
                    execution_path.display().to_string(),
                );

                let interpolated = oj_runbook::interpolate_shell(&cmd, &vars);
                self.logger.append(
                    job_id.as_str(),
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
                            job_id: job_id.clone(),
                            step: step_name.to_string(),
                            agent_id: None,
                            agent_name: None,
                        },
                    },
                    Effect::Shell {
                        owner: Some(oj_core::OwnerId::Job(job_id.clone())),
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
            RunDirective::Agent { agent, .. } => {
                // Idempotency guard: if agent run already exists (e.g., from crash recovery
                // where the CommandRun event is re-processed), skip creation.
                // Use job_id since it's used as the agent_run_id.
                let agent_run_exists =
                    self.lock_state(|s| s.agent_runs.contains_key(job_id.as_str()));
                if agent_run_exists {
                    tracing::debug!(
                        job_id = %job_id,
                        "agent run already exists, skipping duplicate creation"
                    );
                    return Ok(vec![]);
                }

                let agent_name = agent.clone();
                let agent_def = runbook
                    .get_agent(&agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.clone()))?
                    .clone();

                // Check max_concurrency before spawning
                if let Some(max) = agent_def.max_concurrency {
                    let running = self.count_running_agents(&agent_name, namespace);
                    if running >= max as usize {
                        return Err(RuntimeError::InvalidRequest(format!(
                            "agent '{}' at max concurrency ({}/{})",
                            agent_name, running, max
                        )));
                    }
                }

                // Use the job_id as the agent_run_id (daemon generated it)
                let agent_run_id = AgentRunId::new(job_id.to_string());

                // Only pass runbook_json if not already cached
                let already_cached = self.runbook_cache.lock().contains_key(&runbook_hash);
                let mut creation_effects = Vec::new();
                if !already_cached {
                    creation_effects.push(Effect::Emit {
                        event: Event::RunbookLoaded {
                            hash: runbook_hash.clone(),
                            version: 1,
                            runbook: runbook_json,
                        },
                    });
                }

                // Insert into in-process cache
                {
                    self.runbook_cache
                        .lock()
                        .entry(runbook_hash.clone())
                        .or_insert(runbook);
                }

                // Emit AgentRunCreated
                creation_effects.push(Effect::Emit {
                    event: Event::AgentRunCreated {
                        id: agent_run_id.clone(),
                        agent_name: agent_name.clone(),
                        command_name: command.to_string(),
                        namespace: namespace.to_string(),
                        cwd: invoke_dir.to_path_buf(),
                        runbook_hash: runbook_hash.clone(),
                        vars: args.clone(),
                        created_at_epoch_ms: self.clock().epoch_ms(),
                    },
                });

                let mut result_events = self.executor.execute_all(creation_effects).await?;

                // Spawn the standalone agent
                let spawn_events = self
                    .spawn_standalone_agent(SpawnAgentParams {
                        agent_run_id: &agent_run_id,
                        agent_def: &agent_def,
                        agent_name: &agent_name,
                        input: &args,
                        cwd: invoke_dir,
                        namespace,
                        resume_session_id: None,
                    })
                    .await?;
                result_events.extend(spawn_events);

                Ok(result_events)
            }
        }
    }
}
