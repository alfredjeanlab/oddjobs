// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron event handling

use super::super::Runtime;
use super::CreateJobParams;
use crate::error::RuntimeError;
use crate::log_paths::cron_log_path;
use crate::runtime::agent_run::SpawnAgentParams;
use crate::time_fmt::format_utc_now;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{scoped_name, Clock, Effect, Event, IdGen, JobId, ShortId, TimerId, UuidIdGen};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What a cron targets when it fires.
#[derive(Debug, Clone)]
pub(crate) enum CronRunTarget {
    Job(String),
    Agent(String),
}

impl CronRunTarget {
    /// Parse a "job:name" or "agent:name" string.
    pub(crate) fn from_run_target_str(s: &str) -> Self {
        if let Some(name) = s.strip_prefix("agent:") {
            CronRunTarget::Agent(name.to_string())
        } else if let Some(name) = s.strip_prefix("job:") {
            CronRunTarget::Job(name.to_string())
        } else {
            // Backward compat: bare name = job
            CronRunTarget::Job(s.to_string())
        }
    }

    /// Get the display name for logging.
    pub(crate) fn display_name(&self) -> String {
        match self {
            CronRunTarget::Job(name) => format!("job={}", name),
            CronRunTarget::Agent(name) => format!("agent={}", name),
        }
    }
}

/// In-memory state for a running cron
pub(crate) struct CronState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub interval: String,
    pub run_target: CronRunTarget,
    pub status: CronStatus,
    pub namespace: String,
    /// Maximum concurrent jobs this cron can have running. Default 1.
    pub concurrency: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CronStatus {
    Running,
    Stopped,
}

/// Append a timestamped line to the cron log file.
///
/// Creates the `{logs_dir}/cron/` directory on first write.
/// Errors are silently ignored — logging must not break the cron.
fn append_cron_log(logs_dir: &Path, cron_name: &str, namespace: &str, message: &str) {
    let scoped = scoped_name(namespace, cron_name);
    let path = cron_log_path(logs_dir, &scoped);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let ts = format_utc_now();
        let _ = writeln!(f, "[{}] {}", ts, message);
    }
}

/// Parameters for handling a cron started event.
pub(crate) struct CronStartedParams<'a> {
    pub cron_name: &'a str,
    pub project_root: &'a Path,
    pub runbook_hash: &'a str,
    pub interval: &'a str,
    pub run_target: &'a str,
    pub namespace: &'a str,
}

/// Parameters for handling a one-shot cron execution.
pub(crate) struct CronOnceParams<'a> {
    pub cron_name: &'a str,
    pub job_id: &'a JobId,
    pub job_name: &'a str,
    pub job_kind: &'a str,
    pub agent_run_id: &'a Option<String>,
    pub agent_name: &'a Option<String>,
    pub runbook_hash: &'a str,
    pub run_target: &'a str,
    pub namespace: &'a str,
    pub project_root: &'a Path,
}

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

        {
            let mut crons = self.cron_states.lock();
            crons.insert(cron_name.to_string(), state);
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
        {
            let mut crons = self.cron_states.lock();
            if let Some(state) = crons.get_mut(cron_name) {
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

        // Determine target: prefer run_target, fall back to job fields
        let is_agent = if !run_target.is_empty() {
            run_target.starts_with("agent:")
        } else {
            agent_name.is_some()
        };

        if is_agent {
            let agent_name = agent_name
                .as_deref()
                .unwrap_or_else(|| run_target.strip_prefix("agent:").unwrap_or(""));
            let agent_def = runbook
                .get_agent(agent_name)
                .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.to_string()))?
                .clone();

            let ar_id =
                oj_core::AgentRunId::new(agent_run_id.as_deref().unwrap_or(&UuidIdGen.next()));

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

    /// Handle a cron timer firing: spawn job/agent and reschedule timer.
    pub(crate) async fn handle_cron_timer_fired(
        &self,
        rest: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Parse cron name and namespace from timer ID rest (after "cron:" prefix)
        // Format: "cron_name" or "namespace/cron_name"
        let cron_name = rest.rsplit('/').next().unwrap_or(rest);
        let timer_namespace = rest.strip_suffix(&format!("/{}", cron_name)).unwrap_or("");

        let (project_root, runbook_hash, run_target, interval, namespace, concurrency) = {
            let crons = self.cron_states.lock();
            match crons.get(cron_name) {
                Some(s) if s.status == CronStatus::Running => (
                    s.project_root.clone(),
                    s.runbook_hash.clone(),
                    s.run_target.clone(),
                    s.interval.clone(),
                    s.namespace.clone(),
                    s.concurrency,
                ),
                _ => {
                    tracing::debug!(cron = cron_name, "cron timer fired but cron not running");
                    append_cron_log(
                        self.logger.log_dir(),
                        cron_name,
                        timer_namespace,
                        "skip: cron not in running state",
                    );
                    return Ok(vec![]);
                }
            }
        };

        // Refresh runbook from disk
        if let Some(loaded_event) = self.refresh_cron_runbook(cron_name)? {
            // Process the loaded event to update caches
            let _ = self
                .executor
                .execute_all(vec![Effect::Emit {
                    event: loaded_event,
                }])
                .await?;
        }

        // Re-read hash and concurrency after potential refresh
        let (runbook_hash, concurrency) = {
            let crons = self.cron_states.lock();
            crons
                .get(cron_name)
                .map(|s| (s.runbook_hash.clone(), s.concurrency))
                .unwrap_or((runbook_hash, concurrency))
        };

        let runbook = self.cached_runbook(&runbook_hash)?;

        let mut result_events = Vec::new();

        match &run_target {
            CronRunTarget::Job(job_name) => {
                // Check concurrency before spawning
                let active = self.count_active_cron_jobs(cron_name, &namespace);
                if active >= concurrency as usize {
                    append_cron_log(
                        self.logger.log_dir(),
                        cron_name,
                        &namespace,
                        &format!(
                            "skip: job '{}' at max concurrency ({}/{})",
                            job_name, active, concurrency
                        ),
                    );
                    // Reschedule timer but don't spawn
                    let duration = crate::monitor::parse_duration(&interval).map_err(|e| {
                        RuntimeError::InvalidFormat(format!(
                            "invalid cron interval '{}': {}",
                            interval, e
                        ))
                    })?;
                    let timer_id = TimerId::cron(cron_name, &namespace);
                    self.executor
                        .execute(Effect::SetTimer {
                            id: timer_id,
                            duration,
                        })
                        .await?;
                    return Ok(result_events);
                }

                // Generate job ID
                let job_id = JobId::new(UuidIdGen.next());
                let display_name =
                    oj_runbook::job_display_name(job_name, job_id.short(8), &namespace);

                // Set invoke.dir to project root so runbooks can reference ${invoke.dir}
                let mut vars = HashMap::new();
                vars.insert("invoke.dir".to_string(), project_root.display().to_string());

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
                        namespace: namespace.clone(),
                        cron_name: Some(cron_name.to_string()),
                    })
                    .await?,
                );

                append_cron_log(
                    self.logger.log_dir(),
                    cron_name,
                    &namespace,
                    &format!("tick: triggered job {} ({})", job_name, job_id.short(8)),
                );

                // Emit CronFired tracking event
                result_events.extend(
                    self.executor
                        .execute_all(vec![Effect::Emit {
                            event: Event::CronFired {
                                cron_name: cron_name.to_string(),
                                job_id,
                                agent_run_id: None,
                                namespace: namespace.clone(),
                            },
                        }])
                        .await?,
                );
            }
            CronRunTarget::Agent(agent_name) => {
                let agent_def = runbook
                    .get_agent(agent_name)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.clone()))?
                    .clone();

                // Check max_concurrency before spawning
                if let Some(max) = agent_def.max_concurrency {
                    let running = self.count_running_agents(agent_name, &namespace);
                    if running >= max as usize {
                        append_cron_log(
                            self.logger.log_dir(),
                            cron_name,
                            &namespace,
                            &format!(
                                "skip: agent '{}' at max concurrency ({}/{})",
                                agent_name, running, max
                            ),
                        );
                        // Reschedule timer but don't spawn
                        let duration = crate::monitor::parse_duration(&interval).map_err(|e| {
                            RuntimeError::InvalidFormat(format!(
                                "invalid cron interval '{}': {}",
                                interval, e
                            ))
                        })?;
                        let timer_id = TimerId::cron(cron_name, &namespace);
                        self.executor
                            .execute(Effect::SetTimer {
                                id: timer_id,
                                duration,
                            })
                            .await?;
                        return Ok(result_events);
                    }
                }

                let agent_run_id = oj_core::AgentRunId::new(UuidIdGen.next());

                // Emit AgentRunCreated
                let creation_effects = vec![Effect::Emit {
                    event: Event::AgentRunCreated {
                        id: agent_run_id.clone(),
                        agent_name: agent_name.clone(),
                        command_name: format!("cron:{}", cron_name),
                        namespace: namespace.clone(),
                        cwd: project_root.clone(),
                        runbook_hash: runbook_hash.clone(),
                        vars: HashMap::new(),
                        created_at_epoch_ms: self.clock().epoch_ms(),
                    },
                }];
                result_events.extend(self.executor.execute_all(creation_effects).await?);

                // Spawn the standalone agent
                let spawn_events = self
                    .spawn_standalone_agent(SpawnAgentParams {
                        agent_run_id: &agent_run_id,
                        agent_def: &agent_def,
                        agent_name,
                        input: &HashMap::new(),
                        cwd: &project_root,
                        namespace: &namespace,
                        resume_session_id: None,
                    })
                    .await?;
                result_events.extend(spawn_events);

                append_cron_log(
                    self.logger.log_dir(),
                    cron_name,
                    &namespace,
                    &format!(
                        "tick: triggered agent {} ({})",
                        agent_name,
                        agent_run_id.short(8)
                    ),
                );

                // Emit CronFired tracking event
                result_events.extend(
                    self.executor
                        .execute_all(vec![Effect::Emit {
                            event: Event::CronFired {
                                cron_name: cron_name.to_string(),
                                job_id: JobId::new(""),
                                agent_run_id: Some(agent_run_id.as_str().to_string()),
                                namespace: namespace.clone(),
                            },
                        }])
                        .await?,
                );
            }
        }

        // Reschedule timer for next interval
        let duration = crate::monitor::parse_duration(&interval).map_err(|e| {
            RuntimeError::InvalidFormat(format!("invalid cron interval '{}': {}", interval, e))
        })?;
        let timer_id = TimerId::cron(cron_name, &namespace);
        self.executor
            .execute(Effect::SetTimer {
                id: timer_id,
                duration,
            })
            .await?;

        Ok(result_events)
    }

    /// Re-read the runbook from disk for a running cron.
    ///
    /// If the content has changed since last cached, updates the in-process
    /// cache and cron state, and returns a `RunbookLoaded` event for WAL
    /// persistence. Returns `Ok(None)` when the runbook is unchanged.
    fn refresh_cron_runbook(
        &self,
        cron_name: &str,
    ) -> Result<Option<oj_core::Event>, RuntimeError> {
        let (project_root, namespace) = {
            let crons = self.cron_states.lock();
            match crons.get(cron_name) {
                Some(s) => (s.project_root.clone(), s.namespace.clone()),
                None => return Ok(None),
            }
        };

        // Load runbook from disk
        let runbook_dir = project_root.join(".oj/runbooks");
        let runbook = oj_runbook::find_runbook_by_cron(&runbook_dir, cron_name)
            .map_err(|e| {
                let msg = e.to_string();
                append_cron_log(
                    self.logger.log_dir(),
                    cron_name,
                    &namespace,
                    &format!("error: {}", msg),
                );
                RuntimeError::RunbookLoadError(msg)
            })?
            .ok_or_else(|| {
                let msg = format!("no runbook found containing cron '{}'", cron_name);
                append_cron_log(
                    self.logger.log_dir(),
                    cron_name,
                    &namespace,
                    &format!("error: {}", msg),
                );
                RuntimeError::RunbookLoadError(msg)
            })?;

        // Compute content hash
        let runbook_json = serde_json::to_value(&runbook)
            .map_err(|e| RuntimeError::RunbookLoadError(format!("failed to serialize: {}", e)))?;
        let runbook_hash = {
            use sha2::{Digest, Sha256};
            let canonical = serde_json::to_string(&runbook_json).map_err(|e| {
                RuntimeError::RunbookLoadError(format!("failed to serialize: {}", e))
            })?;
            let digest = Sha256::digest(canonical.as_bytes());
            format!("{:x}", digest)
        };

        // Check if hash changed
        let old_hash = {
            let crons = self.cron_states.lock();
            crons
                .get(cron_name)
                .map(|s| s.runbook_hash.clone())
                .unwrap_or_default()
        };

        if old_hash == runbook_hash {
            return Ok(None);
        }

        tracing::info!(
            cron = cron_name,
            old_hash = old_hash.short(12),
            new_hash = runbook_hash.short(12),
            "runbook changed on disk, refreshing"
        );

        // Update cron state (including concurrency from refreshed runbook)
        let new_concurrency = runbook
            .get_cron(cron_name)
            .and_then(|c| c.concurrency)
            .unwrap_or(1);
        {
            let mut crons = self.cron_states.lock();
            if let Some(state) = crons.get_mut(cron_name) {
                state.runbook_hash = runbook_hash.clone();
                state.concurrency = new_concurrency;
            }
        }

        // Update in-process cache
        {
            let mut cache = self.runbook_cache.lock();
            cache.insert(runbook_hash.clone(), runbook);
        }

        // Return RunbookLoaded event for WAL persistence
        Ok(Some(oj_core::Event::RunbookLoaded {
            hash: runbook_hash,
            version: 1,
            runbook: runbook_json,
        }))
    }
}
