// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runtime for the Odd Jobs engine

pub(crate) mod agent_run;
mod handlers;
mod monitor;
mod pipeline;

use crate::{
    breadcrumb::BreadcrumbWriter, error::RuntimeError, pipeline_logger::PipelineLogger,
    queue_logger::QueueLogger, worker_logger::WorkerLogger, Executor, Scheduler,
};
use handlers::cron::CronState;
use handlers::worker::WorkerState;
#[cfg(test)]
use handlers::worker::WorkerStatus;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{AgentId, AgentRunId, Clock, Pipeline, ShortId};
use oj_runbook::Runbook;
use oj_storage::MaterializedState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::mpsc;

// Re-export for tests
#[cfg(test)]
pub use oj_core::{Event, StepStatus};

/// Runtime path configuration
pub struct RuntimeConfig {
    /// Root state directory (e.g. ~/.local/state/oj)
    pub state_dir: PathBuf,
    /// Directory for per-pipeline log files
    pub log_dir: PathBuf,
}

/// Runtime adapter dependencies
pub struct RuntimeDeps<S, A, N> {
    pub sessions: S,
    pub agents: A,
    pub notifier: N,
    pub state: Arc<Mutex<MaterializedState>>,
}

/// Runtime that coordinates the system
pub struct Runtime<S, A, N, C: Clock> {
    pub(crate) executor: Executor<S, A, N, C>,
    pub(crate) state_dir: PathBuf,
    pub(crate) logger: PipelineLogger,
    pub(crate) worker_logger: WorkerLogger,
    pub(crate) queue_logger: QueueLogger,
    pub(crate) breadcrumb: BreadcrumbWriter,
    pub(crate) agent_pipelines: Mutex<HashMap<AgentId, String>>,
    pub(crate) agent_runs: Mutex<HashMap<AgentId, AgentRunId>>,
    pub(crate) runbook_cache: Mutex<HashMap<String, Runbook>>,
    pub(crate) worker_states: Mutex<HashMap<String, WorkerState>>,
    pub(crate) cron_states: Mutex<HashMap<String, CronState>>,
}

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Create a new runtime
    pub fn new(
        deps: RuntimeDeps<S, A, N>,
        clock: C,
        config: RuntimeConfig,
        event_tx: mpsc::Sender<oj_core::Event>,
    ) -> Self {
        Self {
            executor: Executor::new(
                deps,
                Arc::new(Mutex::new(Scheduler::new())),
                clock,
                event_tx,
            ),
            state_dir: config.state_dir,
            logger: PipelineLogger::new(config.log_dir.clone()),
            worker_logger: WorkerLogger::new(config.log_dir.clone()),
            queue_logger: QueueLogger::new(config.log_dir.clone()),
            breadcrumb: BreadcrumbWriter::new(config.log_dir),
            agent_pipelines: Mutex::new(HashMap::new()),
            agent_runs: Mutex::new(HashMap::new()),
            runbook_cache: Mutex::new(HashMap::new()),
            worker_states: Mutex::new(HashMap::new()),
            cron_states: Mutex::new(HashMap::new()),
        }
    }

    /// Get a reference to the clock
    pub fn clock(&self) -> &C {
        self.executor.clock()
    }

    /// Get a shared reference to the scheduler (for timer checking in the daemon loop)
    pub fn scheduler(&self) -> Arc<Mutex<Scheduler>> {
        self.executor.scheduler()
    }

    /// Get current pipelines
    pub fn pipelines(&self) -> HashMap<String, Pipeline> {
        self.lock_state(|state| state.pipelines.clone())
    }

    /// Get a specific pipeline by ID or unique prefix
    pub fn get_pipeline(&self, id: &str) -> Option<Pipeline> {
        self.lock_state(|state| state.get_pipeline(id).cloned())
    }

    /// Helper to lock state and handle poisoned mutex
    pub(crate) fn lock_state<T>(&self, f: impl FnOnce(&MaterializedState) -> T) -> T {
        let state = self.executor.state();
        let guard = state.lock();
        f(&guard)
    }

    /// Helper to lock state mutably and handle poisoned mutex
    pub(crate) fn lock_state_mut<T>(&self, f: impl FnOnce(&mut MaterializedState) -> T) -> T {
        let state = self.executor.state();
        let mut guard = state.lock();
        f(&mut guard)
    }

    /// Count currently active (non-terminal) pipelines spawned by a given cron.
    pub(crate) fn count_active_cron_pipelines(&self, cron_name: &str, namespace: &str) -> usize {
        self.lock_state(|state| {
            state
                .pipelines
                .values()
                .filter(|p| {
                    p.cron_name.as_deref() == Some(cron_name)
                        && p.namespace == namespace
                        && !p.is_terminal()
                })
                .count()
        })
    }

    /// Count currently running (non-terminal) instances of an agent by name.
    pub(crate) fn count_running_agents(&self, agent_name: &str, namespace: &str) -> usize {
        self.lock_state(|state| {
            state
                .agent_runs
                .values()
                .filter(|ar| {
                    ar.agent_name == agent_name
                        && ar.namespace == namespace
                        && !ar.status.is_terminal()
                })
                .count()
        })
    }

    /// Create InvalidRunDirective error
    pub(crate) fn invalid_directive(context: &str, directive: &str, value: &str) -> RuntimeError {
        RuntimeError::InvalidRunDirective {
            context: context.into(),
            directive: format!("{} ({})", directive, value),
        }
    }

    pub(crate) fn require_pipeline(&self, id: &str) -> Result<Pipeline, RuntimeError> {
        self.get_pipeline(id)
            .ok_or_else(|| RuntimeError::PipelineNotFound(id.to_string()))
    }

    pub(crate) fn execution_dir(&self, pipeline: &Pipeline) -> PathBuf {
        // Use workspace_path if in workspace mode, otherwise use cwd
        pipeline
            .workspace_path
            .clone()
            .unwrap_or_else(|| pipeline.cwd.clone())
    }

    /// Load a runbook containing the given command name.
    pub(crate) fn load_runbook_for_command(
        &self,
        project_root: &Path,
        command: &str,
    ) -> Result<Runbook, RuntimeError> {
        let runbook_dir = project_root.join(".oj/runbooks");
        oj_runbook::find_runbook_by_command(&runbook_dir, command)
            .map_err(|e| RuntimeError::RunbookLoadError(e.to_string()))?
            .ok_or_else(|| RuntimeError::CommandNotFound(command.to_string()))
    }

    /// Re-read the runbook from disk for a running worker.
    ///
    /// If the content has changed since last cached, updates the in-process
    /// cache and worker state, and returns a `RunbookLoaded` event for WAL
    /// persistence.  Returns `Ok(None)` when the runbook is unchanged.
    pub(crate) fn refresh_worker_runbook(
        &self,
        worker_name: &str,
    ) -> Result<Option<oj_core::Event>, RuntimeError> {
        let project_root = {
            let workers = self.worker_states.lock();
            match workers.get(worker_name) {
                Some(s) => s.project_root.clone(),
                None => return Ok(None),
            }
        };

        // Load runbook from disk
        let runbook_dir = project_root.join(".oj/runbooks");
        let runbook = oj_runbook::find_runbook_by_worker(&runbook_dir, worker_name)
            .map_err(|e| RuntimeError::RunbookLoadError(e.to_string()))?
            .ok_or_else(|| {
                RuntimeError::RunbookLoadError(format!(
                    "no runbook found containing worker '{}'",
                    worker_name
                ))
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
            let workers = self.worker_states.lock();
            workers
                .get(worker_name)
                .map(|s| s.runbook_hash.clone())
                .unwrap_or_default()
        };

        if old_hash == runbook_hash {
            return Ok(None);
        }

        tracing::info!(
            worker = worker_name,
            old_hash = old_hash.short(12),
            new_hash = runbook_hash.short(12),
            "runbook changed on disk, refreshing"
        );

        // Update worker state
        {
            let mut workers = self.worker_states.lock();
            if let Some(state) = workers.get_mut(worker_name) {
                state.runbook_hash = runbook_hash.clone();
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

    /// Retrieve a cached runbook by content hash.
    ///
    /// Checks the in-process cache first, then falls back to the
    /// materialized state (WAL replay). Populates the cache on miss.
    pub(crate) fn cached_runbook(&self, hash: &str) -> Result<Runbook, RuntimeError> {
        // Check in-process cache
        {
            let cache = self.runbook_cache.lock();
            if let Some(runbook) = cache.get(hash) {
                return Ok(runbook.clone());
            }
        }

        // Cache miss: deserialize from materialized state
        let stored = self.lock_state(|state| state.runbooks.get(hash).cloned());
        let stored = stored.ok_or_else(|| {
            RuntimeError::RunbookLoadError(format!("runbook not found for hash: {}", hash))
        })?;

        let runbook: Runbook = serde_json::from_value(stored.data).map_err(|e| {
            RuntimeError::RunbookLoadError(format!("failed to deserialize stored runbook: {}", e))
        })?;

        // Populate cache
        {
            let mut cache = self.runbook_cache.lock();
            cache.insert(hash.to_string(), runbook.clone());
        }

        Ok(runbook)
    }
}

#[cfg(test)]
#[path = "../runtime_tests/mod.rs"]
mod tests;
