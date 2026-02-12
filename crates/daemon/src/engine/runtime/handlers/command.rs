// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command run event handling

use super::super::Runtime;
use super::CreateJobParams;
use crate::adapters::{AgentAdapter, NotifyAdapter};
use crate::engine::error::RuntimeError;
use crate::engine::runtime::agent::SpawnAgentParams;
use oj_core::{Clock, Effect, Event, OwnerId};
use oj_runbook::RunDirective;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

/// Parameters for handling a command run event.
pub(crate) struct HandleCommandParams<'a> {
    pub owner: &'a OwnerId,
    pub name: &'a str,
    pub project_path: &'a Path,
    pub invoke_dir: &'a Path,
    pub project: &'a str,
    pub command: &'a str,
    pub args: &'a HashMap<String, String>,
}

impl<A, N, C> Runtime<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_command(
        &self,
        params: HandleCommandParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let HandleCommandParams {
            owner,
            name: cmd_name,
            project_path,
            invoke_dir,
            project,
            command,
            args,
        } = params;

        // Load runbook from project
        let runbook = self.load_runbook_for_command(project_path, command)?;

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
        args.entry("invoke.dir".to_string()).or_insert_with(|| invoke_dir.display().to_string());

        let cmd_def = runbook
            .get_command(command)
            .ok_or_else(|| RuntimeError::CommandNotFound(command.to_string()))?;

        match &cmd_def.run {
            RunDirective::Job { .. } => {
                let job_id = owner.try_job()?;

                // Validate job def exists
                let _ = runbook
                    .get_job(cmd_name)
                    .ok_or_else(|| RuntimeError::JobDefNotFound(cmd_name.to_string()))?;

                let name = args.get("name").cloned().unwrap_or_else(|| job_id.to_string());

                // Only pass runbook_json if not already cached
                let already_cached = self.runbook_cache.lock().contains_key(&runbook_hash);
                let runbook_json_param = if already_cached { None } else { Some(runbook_json) };

                self.create_and_start_job(CreateJobParams {
                    job_id: job_id.clone(),
                    job_name: name,
                    job_kind: cmd_name.to_string(),
                    vars: args.clone(),
                    runbook_hash,
                    runbook_json: runbook_json_param,
                    runbook,
                    project: project.to_string(),
                    cron_name: None,
                })
                .await
            }
            RunDirective::Shell(cmd) => {
                let job_id = owner.try_job()?;

                // Idempotency guard: if job already exists (e.g., from crash recovery
                // where the CommandRun event is re-processed), skip creation.
                if self.get_job(job_id.as_str()).is_some() {
                    return Ok(vec![]);
                }

                let cmd = cmd.clone();
                let name = args.get("name").cloned().unwrap_or_else(|| job_id.to_string());
                let step_name = "run";
                let execution_path = project_path.to_path_buf();

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
                        created_at_ms: self.executor.clock().epoch_ms(),
                        project: project.to_string(),
                        cron: None,
                    },
                });

                // Insert into in-process cache
                {
                    self.runbook_cache.lock().entry(runbook_hash).or_insert(runbook);
                }

                let mut result_events = self.executor.execute_all(creation_effects).await?;
                self.logger.append(job_id.as_str(), step_name, "shell command created");

                // Phase 2: Interpolate and execute shell command
                // Values are escaped by interpolate_shell() during substitution
                let mut vars: HashMap<String, String> =
                    args.iter().map(|(k, v)| (format!("args.{}", k), v.clone())).collect();
                vars.insert("job_id".to_string(), job_id.to_string());
                vars.insert("name".to_string(), name.clone());
                vars.insert("workspace".to_string(), execution_path.display().to_string());

                let interpolated = oj_runbook::interpolate_shell(&cmd, &vars);
                self.logger.append(
                    job_id.as_str(),
                    step_name,
                    &format!("shell (cwd: {}): {}", execution_path.display(), interpolated),
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
                        owner: Some(OwnerId::Job(job_id.clone())),
                        step: step_name.to_string(),
                        command: interpolated,
                        cwd: execution_path,
                        env: if project.is_empty() {
                            HashMap::new()
                        } else {
                            HashMap::from([("OJ_PROJECT".to_string(), project.to_string())])
                        },
                        container: None,
                    },
                ];
                result_events.extend(self.executor.execute_all(shell_effects).await?);

                Ok(result_events)
            }
            RunDirective::Agent { agent, .. } => {
                let crew_id = owner.try_crew()?;

                // Idempotency guard: if crew already exists (e.g., from crash recovery
                // where the CommandRun event is re-processed), skip creation.
                let crew_exists = self.lock_state(|s| s.crew.contains_key(crew_id.as_str()));
                if crew_exists {
                    return Ok(vec![]);
                }

                let agent_name = agent.clone();
                let agent_def = runbook
                    .get_agent(&agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.clone()))?
                    .clone();

                // Check max_concurrency before spawning
                if let Some(max) = agent_def.max_concurrency {
                    let running = self.count_running_agents(&agent_name, project);
                    if running >= max as usize {
                        return Err(RuntimeError::InvalidRequest(format!(
                            "agent '{}' at max concurrency ({}/{})",
                            agent_name, running, max
                        )));
                    }
                }

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
                    self.runbook_cache.lock().entry(runbook_hash.clone()).or_insert(runbook);
                }

                // Emit CrewCreated
                creation_effects.push(Effect::Emit {
                    event: Event::CrewCreated {
                        id: crew_id.clone(),
                        agent: agent_name.clone(),
                        command: command.to_string(),
                        project: project.to_string(),
                        cwd: invoke_dir.to_path_buf(),
                        runbook_hash: runbook_hash.clone(),
                        vars: args.clone(),
                        created_at_ms: self.executor.clock().epoch_ms(),
                    },
                });

                let mut result_events = self.executor.execute_all(creation_effects).await?;

                // Spawn the standalone agent
                let spawn_events = self
                    .spawn_standalone_agent(SpawnAgentParams {
                        crew_id,
                        agent_def: &agent_def,
                        agent_name: &agent_name,
                        input: &args,
                        cwd: invoke_dir,
                        project,
                        resume: false,
                    })
                    .await?;
                result_events.extend(spawn_events);

                Ok(result_events)
            }
        }
    }
}
