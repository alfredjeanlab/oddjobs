// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron timer firing, shell job creation, and runbook refresh

use super::{append_cron_log, CronShellJobParams, CronStatus};
use crate::engine::error::RuntimeError;
use crate::engine::runtime::agent::SpawnAgentParams;
use crate::engine::runtime::handlers::CreateJobParams;
use crate::engine::runtime::Runtime;
use oj_core::{split_scoped_name, Clock, Effect, Event, JobId, RunTarget, TimerId};
use std::collections::HashMap;

impl<C: Clock> Runtime<C> {
    /// Handle a cron timer firing: spawn job/agent and reschedule timer.
    pub(crate) async fn handle_cron_timer_fired(
        &self,
        rest: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Timer ID rest is "cron_name" or "project/cron_name" —
        // this is already a scoped key matching the cron_states HashMap key.
        let cron_key = rest;
        let (_, cron_name) = split_scoped_name(cron_key);

        let (project_path, runbook_hash, target, interval, project, concurrency) = {
            let crons = self.cron_states.lock();
            match crons.get(cron_key) {
                Some(s) if s.status == CronStatus::Running => (
                    s.project_path.clone(),
                    s.runbook_hash.clone(),
                    s.target.clone(),
                    s.interval.clone(),
                    s.project.clone(),
                    s.concurrency,
                ),
                _ => {
                    let (timer_ns, _) = split_scoped_name(cron_key);
                    append_cron_log(
                        self.logger.log_dir(),
                        cron_name,
                        timer_ns,
                        "skip: cron not in running state",
                    );
                    return Ok(vec![]);
                }
            }
        };

        // Refresh runbook from disk
        if let Some(loaded_event) = self.refresh_cron_runbook(cron_key)? {
            // Process the loaded event to update caches
            let _ = self.executor.execute_all(vec![Effect::Emit { event: loaded_event }]).await?;
        }

        // Re-read hash and concurrency after potential refresh
        let (runbook_hash, concurrency) = {
            let crons = self.cron_states.lock();
            crons
                .get(cron_key)
                .map(|s| (s.runbook_hash.clone(), s.concurrency))
                .unwrap_or((runbook_hash, concurrency))
        };

        let runbook = self.cached_runbook(&runbook_hash)?;

        let mut result_events = Vec::new();

        match &target {
            RunTarget::Job(job_name) => {
                // Check concurrency before spawning
                let active = self.count_active_cron_jobs(cron_name, &project);
                if active >= concurrency as usize {
                    append_cron_log(
                        self.logger.log_dir(),
                        cron_name,
                        &project,
                        &format!(
                            "skip: job '{}' at max concurrency ({}/{})",
                            job_name, active, concurrency
                        ),
                    );
                    // Reschedule timer but don't spawn
                    let duration =
                        crate::engine::monitor::parse_duration(&interval).map_err(|e| {
                            RuntimeError::InvalidFormat(format!(
                                "invalid cron interval '{}': {}",
                                interval, e
                            ))
                        })?;
                    let timer_id = TimerId::cron(cron_name, &project);
                    self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;
                    return Ok(result_events);
                }

                // Generate job ID
                let job_id = JobId::new();
                let display_name =
                    oj_runbook::job_display_name(job_name, job_id.short(8), &project);

                // Set invoke.dir to project root so runbooks can reference ${invoke.dir}
                let mut vars = HashMap::new();
                vars.insert("invoke.dir".to_string(), project_path.display().to_string());

                // Create and start job
                result_events.extend(
                    self.create_and_start_job(CreateJobParams {
                        job_id: job_id.clone(),
                        job_name: display_name,
                        job_kind: job_name.clone(),
                        vars,
                        runbook_hash: runbook_hash.clone(),
                        runbook_json: None,
                        runbook,
                        project: project.clone(),
                        cron_name: Some(cron_name.to_string()),
                    })
                    .await?,
                );

                append_cron_log(
                    self.logger.log_dir(),
                    cron_name,
                    &project,
                    &format!("tick: triggered job {} ({})", job_name, job_id.short(8)),
                );

                // Emit CronFired tracking event
                result_events.extend(
                    self.executor
                        .execute_all(vec![Effect::Emit {
                            event: Event::CronFired {
                                cron: cron_name.to_string(),
                                owner: job_id.into(),
                                project: project.clone(),
                            },
                        }])
                        .await?,
                );
            }
            RunTarget::Agent(agent_name) => {
                let agent_def = runbook
                    .get_agent(agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.clone()))?
                    .clone();

                // Check max_concurrency before spawning
                if let Some(max) = agent_def.max_concurrency {
                    let running = self.count_running_agents(agent_name, &project);
                    if running >= max as usize {
                        append_cron_log(
                            self.logger.log_dir(),
                            cron_name,
                            &project,
                            &format!(
                                "skip: agent '{}' at max concurrency ({}/{})",
                                agent_name, running, max
                            ),
                        );
                        // Reschedule timer but don't spawn
                        let duration =
                            crate::engine::monitor::parse_duration(&interval).map_err(|e| {
                                RuntimeError::InvalidFormat(format!(
                                    "invalid cron interval '{}': {}",
                                    interval, e
                                ))
                            })?;
                        let timer_id = TimerId::cron(cron_name, &project);
                        self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;
                        return Ok(result_events);
                    }
                }

                let crew_id = oj_core::CrewId::new();

                // Emit CrewCreated
                let creation_effects = vec![Effect::Emit {
                    event: Event::CrewCreated {
                        id: crew_id.clone(),
                        agent: agent_name.clone(),
                        command: format!("cron:{}", cron_name),
                        project: project.clone(),
                        cwd: project_path.clone(),
                        runbook_hash: runbook_hash.clone(),
                        vars: HashMap::new(),
                        created_at_ms: self.executor.clock().epoch_ms(),
                    },
                }];
                result_events.extend(self.executor.execute_all(creation_effects).await?);

                // Spawn the standalone agent
                let spawn_events = self
                    .spawn_standalone_agent(SpawnAgentParams {
                        crew_id: &crew_id,
                        agent_def: &agent_def,
                        agent_name,
                        input: &HashMap::new(),
                        cwd: &project_path,
                        project: &project,
                        resume: false,
                    })
                    .await?;
                result_events.extend(spawn_events);

                append_cron_log(
                    self.logger.log_dir(),
                    cron_name,
                    &project,
                    &format!("tick: triggered agent {} ({})", agent_name, crew_id.short(8)),
                );

                // Emit CronFired tracking event
                result_events.extend(
                    self.executor
                        .execute_all(vec![Effect::Emit {
                            event: Event::CronFired {
                                cron: cron_name.to_string(),
                                owner: crew_id.into(),
                                project: project.clone(),
                            },
                        }])
                        .await?,
                );
            }
            RunTarget::Shell(cmd) => {
                // Check concurrency before spawning
                let active = self.count_active_cron_jobs(cron_name, &project);
                if active >= concurrency as usize {
                    append_cron_log(
                        self.logger.log_dir(),
                        cron_name,
                        &project,
                        &format!("skip: shell at max concurrency ({}/{})", active, concurrency),
                    );
                    // Reschedule timer but don't spawn
                    let duration =
                        crate::engine::monitor::parse_duration(&interval).map_err(|e| {
                            RuntimeError::InvalidFormat(format!(
                                "invalid cron interval '{}': {}",
                                interval, e
                            ))
                        })?;
                    let timer_id = TimerId::cron(cron_name, &project);
                    self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;
                    return Ok(result_events);
                }

                let job_id = JobId::new();
                let display_name =
                    oj_runbook::job_display_name(cron_name, job_id.short(8), &project);

                result_events.extend(
                    self.create_cron_shell_job(CronShellJobParams {
                        job_id: job_id.clone(),
                        cron: cron_name,
                        job_display: &display_name,
                        cmd,
                        runbook_hash: &runbook_hash,
                        project: &project,
                        cwd: &project_path,
                    })
                    .await?,
                );

                append_cron_log(
                    self.logger.log_dir(),
                    cron_name,
                    &project,
                    &format!("tick: triggered shell ({})", job_id.short(8)),
                );
            }
        }

        // Reschedule timer for next interval
        let duration = crate::engine::monitor::parse_duration(&interval).map_err(|e| {
            RuntimeError::InvalidFormat(format!("invalid cron interval '{}': {}", interval, e))
        })?;
        let timer_id = TimerId::cron(cron_name, &project);
        self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;

        Ok(result_events)
    }

    /// Create an inline job with a single shell step for a cron target.
    pub(crate) async fn create_cron_shell_job(
        &self,
        params: CronShellJobParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let CronShellJobParams { job_id, job_display, runbook_hash, project, cron, cmd, cwd } =
            params;
        let step_name = "run";
        let execution_path = cwd.to_path_buf();

        let creation_effects = vec![Effect::Emit {
            event: Event::JobCreated {
                id: job_id.clone(),
                kind: cron.to_string(),
                name: job_display.to_string(),
                runbook_hash: runbook_hash.to_string(),
                cwd: execution_path.clone(),
                vars: HashMap::new(),
                initial_step: step_name.to_string(),
                created_at_ms: self.executor.clock().epoch_ms(),
                project: project.to_string(),
                cron: Some(cron.to_string()),
            },
        }];
        let mut result_events = self.executor.execute_all(creation_effects).await?;

        let mut vars = HashMap::new();
        vars.insert("job_id".to_string(), job_id.to_string());
        vars.insert("workspace".to_string(), execution_path.display().to_string());
        let interpolated = oj_runbook::interpolate_shell(cmd, &vars);

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
                env: if project.is_empty() {
                    HashMap::new()
                } else {
                    HashMap::from([("OJ_PROJECT".to_string(), project.to_string())])
                },
                container: None,
            },
        ];
        result_events.extend(self.executor.execute_all(shell_effects).await?);

        // Emit CronFired tracking event
        result_events.extend(
            self.executor
                .execute_all(vec![Effect::Emit {
                    event: Event::CronFired {
                        cron: cron.to_string(),
                        owner: job_id.into(),
                        project: project.to_string(),
                    },
                }])
                .await?,
        );

        Ok(result_events)
    }
}
