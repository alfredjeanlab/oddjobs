// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron event handling: start, stop, and one-shot execution

use super::append_cron_log;
use crate::error::RuntimeError;
use crate::runtime::agent::SpawnAgentParams;
use crate::runtime::handlers::CreateJobParams;
use crate::runtime::Runtime;
use oj_adapters::{AgentAdapter, NotifyAdapter};
use oj_core::{scoped_name, Clock, Effect, Event, IdGen, JobId, TimerId, UuidIdGen};
use std::collections::HashMap;

pub(crate) use super::{CronOnceParams, CronStartedParams};
use super::{CronShellJobParams, CronState, CronStatus};

impl<A, N, C> Runtime<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_cron_started(
        &self,
        params: CronStartedParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let CronStartedParams { cron, project, project_path, runbook_hash, interval, target } =
            params;
        let duration = crate::monitor::parse_duration(interval).map_err(|e| {
            RuntimeError::InvalidFormat(format!("invalid cron interval '{}': {}", interval, e))
        })?;

        // Read concurrency from the cron definition in the runbook
        let concurrency = self
            .cached_runbook(runbook_hash)
            .ok()
            .and_then(|rb| rb.get_cron(cron).map(|c| c.concurrency.unwrap_or(1)))
            .unwrap_or(1);

        // Store cron state
        let state = CronState {
            project_path: project_path.to_path_buf(),
            runbook_hash: runbook_hash.to_string(),
            interval: interval.to_string(),
            target: target.clone(),
            status: CronStatus::Running,
            project: project.to_string(),
            concurrency,
        };

        let cron_key = scoped_name(project, cron);
        {
            let mut crons = self.cron_states.lock();
            crons.insert(cron_key, state);
        }

        // Set the first interval timer
        let timer_id = TimerId::cron(cron, project);
        self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;

        append_cron_log(
            self.logger.log_dir(),
            cron,
            project,
            &format!("started (interval={}, {})", interval, target.log()),
        );

        Ok(vec![])
    }

    pub(crate) async fn handle_cron_stopped(
        &self,
        cron_name: &str,
        project: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let cron_key = scoped_name(project, cron_name);
        {
            let mut crons = self.cron_states.lock();
            if let Some(state) = crons.get_mut(&cron_key) {
                state.status = CronStatus::Stopped;
            }
        }

        // Cancel the timer
        let timer_id = TimerId::cron(cron_name, project);
        self.executor.execute(Effect::CancelTimer { id: timer_id }).await?;

        append_cron_log(self.logger.log_dir(), cron_name, project, "stopped");

        Ok(vec![])
    }

    /// Handle a one-shot cron execution: create and start the job/agent immediately.
    pub(crate) async fn handle_cron_once(
        &self,
        params: CronOnceParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let CronOnceParams { cron, owner, runbook_hash, target, project, project_path } = params;
        let runbook = self.cached_runbook(runbook_hash)?;
        let mut result_events = Vec::new();

        match target {
            oj_core::RunTarget::Shell(cmd) => {
                let job_id = owner.try_job()?;

                // Idempotency guard
                if self.get_job(job_id.as_str()).is_some() {
                    return Ok(vec![]);
                }

                let display = oj_runbook::job_display_name(cron, job_id.short(8), project);
                result_events.extend(
                    self.create_cron_shell_job(CronShellJobParams {
                        job_id: job_id.clone(),
                        cron,
                        job_display: &display,
                        cmd,
                        runbook_hash,
                        project,
                        cwd: project_path,
                    })
                    .await?,
                );
            }
            oj_core::RunTarget::Agent(agent_name) => {
                let agent_def = runbook
                    .get_agent(agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.to_string()))?
                    .clone();

                let crew_id = owner.try_crew()?;

                // Idempotency guard: if crew already exists (e.g., from crash recovery
                // where the CronOnce event is re-processed), skip creation.
                let crew_exists = self.lock_state(|s| s.crew.contains_key(crew_id.as_str()));
                if crew_exists {
                    return Ok(vec![]);
                }

                // Emit CrewCreated
                let creation_effects = vec![Effect::Emit {
                    event: Event::CrewCreated {
                        id: crew_id.clone(),
                        agent: agent_name.to_string(),
                        command: format!("cron:{}", cron),
                        project: project.to_string(),
                        cwd: project_path.to_path_buf(),
                        runbook_hash: runbook_hash.to_string(),
                        vars: HashMap::new(),
                        created_at_ms: self.executor.clock().epoch_ms(),
                    },
                }];
                result_events.extend(self.executor.execute_all(creation_effects).await?);

                let spawn_events = self
                    .spawn_standalone_agent(SpawnAgentParams {
                        crew_id,
                        agent_def: &agent_def,
                        agent_name,
                        input: &HashMap::new(),
                        cwd: project_path,
                        project,
                        resume: false,
                    })
                    .await?;
                result_events.extend(spawn_events);

                // Emit CronFired tracking event
                result_events.extend(
                    self.executor
                        .execute_all(vec![Effect::Emit {
                            event: Event::CronFired {
                                cron: cron.to_string(),
                                owner: crew_id.into(),
                                project: project.to_string(),
                            },
                        }])
                        .await?,
                );
            }
            oj_core::RunTarget::Job(job_name) => {
                let job_id =
                    owner.as_job().cloned().unwrap_or_else(|| JobId::new(UuidIdGen.next()));

                let display_name = oj_runbook::job_display_name(job_name, job_id.short(8), project);

                // Set invoke.dir to project root so runbooks can reference ${invoke.dir}
                let mut vars = HashMap::new();
                vars.insert("invoke.dir".to_string(), project_path.display().to_string());

                result_events.extend(
                    self.create_and_start_job(CreateJobParams {
                        job_id: job_id.clone(),
                        job_name: display_name,
                        job_kind: job_name.to_string(),
                        vars,
                        runbook_hash: runbook_hash.to_string(),
                        runbook_json: None,
                        runbook,
                        project: project.to_string(),
                        cron_name: Some(cron.to_string()),
                    })
                    .await?,
                );

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
            }
        }

        Ok(result_events)
    }
}
