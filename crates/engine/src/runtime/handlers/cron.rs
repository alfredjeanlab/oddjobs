// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron event handling: start, stop, and one-shot execution

use super::super::Runtime;
use super::cron_types::append_cron_log;
use super::CreateJobParams;
use crate::error::RuntimeError;
use crate::runtime::agent_run::SpawnAgentParams;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{scoped_name, Clock, Effect, Event, IdGen, JobId, TimerId, UuidIdGen};
use std::collections::HashMap;

pub(crate) use super::cron_types::{
    CronOnceParams, CronRunTarget, CronShellJobParams, CronStartedParams, CronState, CronStatus,
};

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_cron_started(
        &self,
        params: CronStartedParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let CronStartedParams {
            cron_name,
            project_root,
            runbook_hash,
            interval,
            run_target: run_target_str,
            namespace,
        } = params;
        let duration = crate::monitor::parse_duration(interval).map_err(|e| {
            RuntimeError::InvalidFormat(format!("invalid cron interval '{}': {}", interval, e))
        })?;

        let run_target = CronRunTarget::from_run_target_str(run_target_str);

        // Read concurrency from the cron definition in the runbook
        let concurrency = self
            .cached_runbook(runbook_hash)
            .ok()
            .and_then(|rb| rb.get_cron(cron_name).map(|c| c.concurrency.unwrap_or(1)))
            .unwrap_or(1);

        // Store cron state
        let state = CronState {
            project_root: project_root.to_path_buf(),
            runbook_hash: runbook_hash.to_string(),
            interval: interval.to_string(),
            run_target: run_target.clone(),
            status: CronStatus::Running,
            namespace: namespace.to_string(),
            concurrency,
        };

        let cron_key = scoped_name(namespace, cron_name);
        {
            let mut crons = self.cron_states.lock();
            crons.insert(cron_key, state);
        }

        // Set the first interval timer
        let timer_id = TimerId::cron(cron_name, namespace);
        self.executor
            .execute(Effect::SetTimer {
                id: timer_id,
                duration,
            })
            .await?;

        append_cron_log(
            self.logger.log_dir(),
            cron_name,
            namespace,
            &format!(
                "started (interval={}, {})",
                interval,
                run_target.display_name()
            ),
        );

        Ok(vec![])
    }

    pub(crate) async fn handle_cron_stopped(
        &self,
        cron_name: &str,
        namespace: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let cron_key = scoped_name(namespace, cron_name);
        {
            let mut crons = self.cron_states.lock();
            if let Some(state) = crons.get_mut(&cron_key) {
                state.status = CronStatus::Stopped;
            }
        }

        // Cancel the timer
        let timer_id = TimerId::cron(cron_name, namespace);
        self.executor
            .execute(Effect::CancelTimer { id: timer_id })
            .await?;

        append_cron_log(self.logger.log_dir(), cron_name, namespace, "stopped");

        Ok(vec![])
    }

    /// Handle a one-shot cron execution: create and start the job/agent immediately.
    pub(crate) async fn handle_cron_once(
        &self,
        params: CronOnceParams<'_>,
    ) -> Result<Vec<Event>, RuntimeError> {
        let CronOnceParams {
            cron_name,
            job_id,
            job_name,
            job_kind,
            agent_run_id,
            agent_name,
            runbook_hash,
            run_target,
            namespace,
            project_root,
        } = params;
        let runbook = self.cached_runbook(runbook_hash)?;
        let mut result_events = Vec::new();

        // Determine target type from run_target string
        let is_agent = if !run_target.is_empty() {
            run_target.starts_with("agent:")
        } else {
            agent_name.is_some()
        };
        let is_shell = run_target.starts_with("shell:");

        if is_shell {
            let cmd = run_target.strip_prefix("shell:").unwrap_or("");

            // Idempotency guard
            if self.get_job(job_id.as_str()).is_some() {
                tracing::debug!(
                    job_id = %job_id,
                    "job already exists, skipping duplicate cron shell creation"
                );
                return Ok(vec![]);
            }

            let display = oj_runbook::job_display_name(cron_name, job_id.short(8), namespace);
            result_events.extend(
                self.create_cron_shell_job(CronShellJobParams {
                    job_id: job_id.clone(),
                    cron_name,
                    job_display: &display,
                    cmd,
                    runbook_hash,
                    namespace,
                    cwd: project_root,
                })
                .await?,
            );
        } else if is_agent {
            let agent_name = agent_name
                .as_deref()
                .unwrap_or_else(|| run_target.strip_prefix("agent:").unwrap_or(""));
            let agent_def = runbook
                .get_agent(agent_name)
                .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.to_string()))?
                .clone();

            let ar_id =
                oj_core::AgentRunId::new(agent_run_id.as_deref().unwrap_or(&UuidIdGen.next()));

            // Idempotency guard: if agent run already exists (e.g., from crash recovery
            // where the CronOnce event is re-processed), skip creation.
            let agent_run_exists = self.lock_state(|s| s.agent_runs.contains_key(ar_id.as_str()));
            if agent_run_exists {
                tracing::debug!(
                    agent_run_id = %ar_id,
                    cron_name,
                    "agent run already exists, skipping duplicate cron agent creation"
                );
                return Ok(vec![]);
            }

            // Emit AgentRunCreated
            let creation_effects = vec![Effect::Emit {
                event: Event::AgentRunCreated {
                    id: ar_id.clone(),
                    agent_name: agent_name.to_string(),
                    command_name: format!("cron:{}", cron_name),
                    namespace: namespace.to_string(),
                    cwd: project_root.to_path_buf(),
                    runbook_hash: runbook_hash.to_string(),
                    vars: HashMap::new(),
                    created_at_epoch_ms: self.clock().epoch_ms(),
                },
            }];
            result_events.extend(self.executor.execute_all(creation_effects).await?);

            let spawn_events = self
                .spawn_standalone_agent(SpawnAgentParams {
                    agent_run_id: &ar_id,
                    agent_def: &agent_def,
                    agent_name,
                    input: &HashMap::new(),
                    cwd: project_root,
                    namespace,
                    resume_session_id: None,
                })
                .await?;
            result_events.extend(spawn_events);

            // Emit CronFired tracking event
            result_events.extend(
                self.executor
                    .execute_all(vec![Effect::Emit {
                        event: Event::CronFired {
                            cron_name: cron_name.to_string(),
                            job_id: JobId::new(""),
                            agent_run_id: Some(ar_id.as_str().to_string()),
                            namespace: namespace.to_string(),
                        },
                    }])
                    .await?,
            );
        } else {
            // Set invoke.dir to project root so runbooks can reference ${invoke.dir}
            let mut vars = HashMap::new();
            vars.insert("invoke.dir".to_string(), project_root.display().to_string());

            // Job target (original behavior)
            result_events.extend(
                self.create_and_start_job(CreateJobParams {
                    job_id: job_id.clone(),
                    job_name: job_name.to_string(),
                    job_kind: job_kind.to_string(),
                    vars,
                    runbook_hash: runbook_hash.to_string(),
                    runbook_json: None,
                    runbook,
                    namespace: namespace.to_string(),
                    cron_name: Some(cron_name.to_string()),
                })
                .await?,
            );

            // Emit CronFired tracking event
            result_events.extend(
                self.executor
                    .execute_all(vec![Effect::Emit {
                        event: Event::CronFired {
                            cron_name: cron_name.to_string(),
                            job_id: job_id.clone(),
                            agent_run_id: None,
                            namespace: namespace.to_string(),
                        },
                    }])
                    .await?,
            );
        }

        Ok(result_events)
    }
}
