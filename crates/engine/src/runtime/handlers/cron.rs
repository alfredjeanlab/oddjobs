// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron event handling

use super::super::Runtime;
use super::CreatePipelineParams;
use crate::error::RuntimeError;
use crate::log_paths::cron_log_path;
use crate::time_fmt::format_utc_now;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, IdGen, PipelineId, TimerId, UuidIdGen};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What a cron targets when it fires.
#[derive(Debug, Clone)]
pub(crate) enum CronRunTarget {
    Pipeline(String),
    Agent(String),
}

impl CronRunTarget {
    /// Parse a "pipeline:name" or "agent:name" string.
    pub(crate) fn from_run_target_str(s: &str) -> Self {
        if let Some(name) = s.strip_prefix("agent:") {
            CronRunTarget::Agent(name.to_string())
        } else if let Some(name) = s.strip_prefix("pipeline:") {
            CronRunTarget::Pipeline(name.to_string())
        } else {
            // Backward compat: bare name = pipeline
            CronRunTarget::Pipeline(s.to_string())
        }
    }

    /// Get the display name for logging.
    pub(crate) fn display_name(&self) -> String {
        match self {
            CronRunTarget::Pipeline(name) => format!("pipeline={}", name),
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
    /// Maximum concurrent pipelines this cron can have running. Default 1.
    pub concurrency: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CronStatus {
    Running,
    Stopped,
}

/// Build a namespace-scoped cron name for log file paths.
fn scoped_cron_name(namespace: &str, cron_name: &str) -> String {
    if namespace.is_empty() {
        cron_name.to_string()
    } else {
        format!("{}/{}", namespace, cron_name)
    }
}

/// Append a timestamped line to the cron log file.
///
/// Creates the `{logs_dir}/cron/` directory on first write.
/// Errors are silently ignored — logging must not break the cron.
fn append_cron_log(logs_dir: &Path, cron_name: &str, namespace: &str, message: &str) {
    let scoped = scoped_cron_name(namespace, cron_name);
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

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    // TODO(refactor): group cron handler parameters into a struct
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_cron_started(
        &self,
        cron_name: &str,
        project_root: &Path,
        runbook_hash: &str,
        interval: &str,
        pipeline_name: &str,
        run_target_str: &str,
        namespace: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let duration = crate::monitor::parse_duration(interval).map_err(|e| {
            RuntimeError::InvalidFormat(format!("invalid cron interval '{}': {}", interval, e))
        })?;

        // Resolve run target from run_target string, falling back to pipeline_name
        let run_target = if run_target_str.is_empty() {
            CronRunTarget::Pipeline(pipeline_name.to_string())
        } else {
            CronRunTarget::from_run_target_str(run_target_str)
        };

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

    /// Handle a one-shot cron execution: create and start the pipeline/agent immediately.
    // TODO(refactor): group cron handler parameters into a struct
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_cron_once(
        &self,
        cron_name: &str,
        pipeline_id: &PipelineId,
        pipeline_name: &str,
        pipeline_kind: &str,
        agent_run_id: &Option<String>,
        agent_name: &Option<String>,
        runbook_hash: &str,
        run_target: &str,
        namespace: &str,
        project_root: &Path,
    ) -> Result<Vec<Event>, RuntimeError> {
        let runbook = self.cached_runbook(runbook_hash)?;
        let mut result_events = Vec::new();

        // Determine target: prefer run_target, fall back to pipeline fields
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
                .spawn_standalone_agent(
                    &ar_id,
                    &agent_def,
                    agent_name,
                    &HashMap::new(),
                    project_root,
                    namespace,
                )
                .await?;
            result_events.extend(spawn_events);

            // Emit CronFired tracking event
            result_events.extend(
                self.executor
                    .execute_all(vec![Effect::Emit {
                        event: Event::CronFired {
                            cron_name: cron_name.to_string(),
                            pipeline_id: PipelineId::new(""),
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

            // Pipeline target (original behavior)
            result_events.extend(
                self.create_and_start_pipeline(CreatePipelineParams {
                    pipeline_id: pipeline_id.clone(),
                    pipeline_name: pipeline_name.to_string(),
                    pipeline_kind: pipeline_kind.to_string(),
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
                            pipeline_id: pipeline_id.clone(),
                            agent_run_id: None,
                            namespace: namespace.to_string(),
                        },
                    }])
                    .await?,
            );
        }

        Ok(result_events)
    }

    /// Handle a cron timer firing: spawn pipeline/agent and reschedule timer.
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
            CronRunTarget::Pipeline(pipeline_name) => {
                // Check concurrency before spawning
                let active = self.count_active_cron_pipelines(cron_name, &namespace);
                if active >= concurrency as usize {
                    append_cron_log(
                        self.logger.log_dir(),
                        cron_name,
                        &namespace,
                        &format!(
                            "skip: pipeline '{}' at max concurrency ({}/{})",
                            pipeline_name, active, concurrency
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

                // Generate pipeline ID
                let pipeline_id = PipelineId::new(UuidIdGen.next());
                let display_name = oj_runbook::pipeline_display_name(
                    pipeline_name,
                    &pipeline_id.as_str()[..8.min(pipeline_id.as_str().len())],
                    &namespace,
                );

                // Set invoke.dir to project root so runbooks can reference ${invoke.dir}
                let mut vars = HashMap::new();
                vars.insert("invoke.dir".to_string(), project_root.display().to_string());

                // Create and start pipeline
                result_events.extend(
                    self.create_and_start_pipeline(CreatePipelineParams {
                        pipeline_id: pipeline_id.clone(),
                        pipeline_name: display_name,
                        pipeline_kind: pipeline_name.clone(),
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
                    &format!(
                        "tick: triggered pipeline {} ({})",
                        pipeline_name,
                        &pipeline_id.as_str()[..8.min(pipeline_id.as_str().len())]
                    ),
                );

                // Emit CronFired tracking event
                result_events.extend(
                    self.executor
                        .execute_all(vec![Effect::Emit {
                            event: Event::CronFired {
                                cron_name: cron_name.to_string(),
                                pipeline_id,
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
                    .spawn_standalone_agent(
                        &agent_run_id,
                        &agent_def,
                        agent_name,
                        &HashMap::new(),
                        &project_root,
                        &namespace,
                    )
                    .await?;
                result_events.extend(spawn_events);

                append_cron_log(
                    self.logger.log_dir(),
                    cron_name,
                    &namespace,
                    &format!(
                        "tick: triggered agent {} ({})",
                        agent_name,
                        &agent_run_id.as_str()[..8.min(agent_run_id.as_str().len())]
                    ),
                );

                // Emit CronFired tracking event
                result_events.extend(
                    self.executor
                        .execute_all(vec![Effect::Emit {
                            event: Event::CronFired {
                                cron_name: cron_name.to_string(),
                                pipeline_id: PipelineId::new(""),
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
            old_hash = &old_hash[..12.min(old_hash.len())],
            new_hash = &runbook_hash[..12.min(runbook_hash.len())],
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
