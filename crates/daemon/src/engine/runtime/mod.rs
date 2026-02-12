// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runtime for the Odd Jobs engine

pub(crate) mod agent;
mod gate;
mod handlers;
mod job;
mod monitor;
mod signal;

use crate::adapters::{AgentAdapter, NotifyAdapter};
use crate::engine::{
    activity_logger::{JobLogger, QueueLogger, WorkerLogger},
    breadcrumb::BreadcrumbWriter,
    error::RuntimeError,
    executor::Executor,
    scheduler::Scheduler,
};
use handlers::cron::CronState;
use handlers::worker::WorkerState;
#[cfg(test)]
use handlers::worker::WorkerStatus;
use oj_core::actions::ActionTracker;
use oj_core::{AgentId, Clock, Crew, Job, OwnerId};
use oj_runbook::Runbook;

use crate::storage::MaterializedState;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::mpsc;

#[cfg(test)]
use oj_core::{Event, StepStatus};

/// Runtime path configuration
pub struct RuntimeConfig {
    /// Root state directory (e.g. ~/.local/state/oj)
    pub state_dir: PathBuf,
    /// Directory for per-job log files
    pub log_dir: PathBuf,
}

/// Mutable references to fields shared between Job and Crew.
/// Used by `with_run_mut` to avoid duplicating OwnerId dispatch logic.
struct RunState<'a> {
    actions: &'a mut ActionTracker,
    last_nudge_at: &'a mut Option<u64>,
}

/// Runtime adapter dependencies
pub struct RuntimeDeps<A, N> {
    pub agents: A,
    pub notifier: N,
    pub state: Arc<Mutex<MaterializedState>>,
}

/// Runtime that coordinates the system
pub struct Runtime<A, N, C: Clock> {
    pub executor: Executor<A, N, C>,
    pub(crate) state_dir: PathBuf,
    pub(crate) logger: JobLogger,
    pub(crate) worker_logger: WorkerLogger,
    pub(crate) queue_logger: QueueLogger,
    pub(crate) breadcrumb: BreadcrumbWriter,
    pub(crate) agent_owners: Mutex<HashMap<AgentId, OwnerId>>,
    pub(crate) runbook_cache: Mutex<HashMap<String, Runbook>>,
    pub(crate) worker_states: Mutex<HashMap<String, WorkerState>>,
    pub(crate) cron_states: Mutex<HashMap<String, CronState>>,
}

impl<A, N, C> Runtime<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Create a new runtime
    pub fn new(
        deps: RuntimeDeps<A, N>,
        clock: C,
        config: RuntimeConfig,
        event_tx: mpsc::Sender<oj_core::Event>,
    ) -> Self {
        Self {
            executor: Executor::new(deps, Arc::new(Mutex::new(Scheduler::new())), clock, event_tx),
            state_dir: config.state_dir,
            logger: JobLogger::new(config.log_dir.clone()),
            worker_logger: WorkerLogger::new(config.log_dir.clone()),
            queue_logger: QueueLogger::new(config.log_dir.clone()),
            breadcrumb: BreadcrumbWriter::new(config.log_dir),
            agent_owners: Mutex::new(HashMap::new()),
            runbook_cache: Mutex::new(HashMap::new()),
            worker_states: Mutex::new(HashMap::new()),
            cron_states: Mutex::new(HashMap::new()),
        }
    }

    /// Get current jobs
    // NOTE(lifetime): used in tests
    #[allow(dead_code)]
    pub fn jobs(&self) -> HashMap<String, Job> {
        self.lock_state(|state| state.jobs.clone())
    }

    /// Get a specific job by ID or unique prefix
    pub fn get_job(&self, id: &str) -> Option<Job> {
        self.lock_state(|state| state.get_job(id).cloned())
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

    /// Count currently active (non-terminal) jobs spawned by a given cron.
    pub(crate) fn count_active_cron_jobs(&self, cron_name: &str, project: &str) -> usize {
        self.lock_state(|state| {
            state
                .jobs
                .values()
                .filter(|p| {
                    p.cron_name.as_deref() == Some(cron_name)
                        && p.project == project
                        && !p.is_terminal()
                })
                .count()
        })
    }

    /// Count currently running (non-terminal) instances of an agent by name.
    pub(crate) fn count_running_agents(&self, agent_name: &str, project: &str) -> usize {
        self.lock_state(|state| {
            state
                .crew
                .values()
                .filter(|run| {
                    run.agent_name == agent_name
                        && run.project == project
                        && !run.status.is_terminal()
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

    pub(crate) fn require_job(&self, id: &str) -> Result<Job, RuntimeError> {
        self.get_job(id).ok_or_else(|| RuntimeError::JobNotFound(id.to_string()))
    }

    pub(crate) fn require_crew(&self, id: &str) -> Result<Crew, RuntimeError> {
        self.lock_state(|s| s.crew.get(id).cloned())
            .ok_or_else(|| RuntimeError::InvalidRequest(format!("crew {} not found", id)))
    }

    /// Get a job by ID, returning None if not found or if the job is terminal.
    ///
    /// This consolidates the common pattern of:
    /// 1. Look up job by ID
    /// 2. Return early if not found
    /// 3. Return early if job is terminal (done/failed/cancelled)
    pub(crate) fn get_active_job(&self, id: &str) -> Option<Job> {
        self.get_job(id).filter(|job| !job.is_terminal())
    }

    /// Resolve a non-terminal run for the given owner.
    pub(crate) fn get_active_run(
        &self,
        owner: &OwnerId,
    ) -> Option<Box<dyn crate::engine::lifecycle::RunLifecycle>> {
        match owner {
            OwnerId::Job(job_id) => {
                let job = self.get_active_job(job_id.as_str())?;
                Some(Box::new(job))
            }
            OwnerId::Crew(run_id) => {
                let run = self.lock_state(|s| s.crew.get(run_id.as_str()).cloned())?;
                if run.status.is_terminal() {
                    return None;
                }
                Some(Box::new(run))
            }
        }
    }

    /// Look up the owner of an agent.
    pub(crate) fn get_agent_owner(&self, agent_id: &AgentId) -> Option<OwnerId> {
        self.agent_owners.lock().get(agent_id).cloned()
    }

    /// Register an agent with its owner.
    pub fn register_agent(&self, agent_id: AgentId, owner: OwnerId) {
        self.agent_owners.lock().insert(agent_id, owner);
    }

    /// Resolve mutable references to the shared run fields for the given owner.
    fn with_run_mut<R>(&self, owner: &OwnerId, f: impl FnOnce(RunState<'_>) -> R) -> Option<R> {
        self.lock_state_mut(|state| match owner {
            OwnerId::Job(job_id) => state.jobs.get_mut(job_id.as_str()).map(|j| {
                f(RunState { actions: &mut j.actions, last_nudge_at: &mut j.last_nudge_at })
            }),
            OwnerId::Crew(crew_id) => state.crew.get_mut(crew_id.as_str()).map(|crew| {
                f(RunState { actions: &mut crew.actions, last_nudge_at: &mut crew.last_nudge_at })
            }),
        })
    }

    pub(crate) fn increment_run_attempt(
        &self,
        owner: &OwnerId,
        trigger: &str,
        chain_pos: usize,
    ) -> u32 {
        self.with_run_mut(owner, |e| e.actions.increment_attempt(trigger, chain_pos)).unwrap_or(1)
    }

    pub(crate) fn reset_run_attempts(&self, owner: &OwnerId) {
        self.with_run_mut(owner, |e| e.actions.reset_attempts());
    }

    pub(crate) fn set_run_nudge_at(&self, owner: &OwnerId, epoch_ms: u64) {
        self.with_run_mut(owner, |e| *e.last_nudge_at = Some(epoch_ms));
    }

    /// Look up the source of a pending (unresolved) decision for an owner.
    /// Returns None if no pending decision exists.
    pub(crate) fn pending_decision_source(
        &self,
        owner: &OwnerId,
    ) -> Option<(oj_core::DecisionId, oj_core::DecisionSource)> {
        self.lock_state(|state| {
            state
                .decisions
                .values()
                .find(|d| d.owner == *owner && !d.is_resolved())
                .map(|d| (d.id.clone(), d.source.clone()))
        })
    }

    /// Load a runbook containing the given command name.
    pub(crate) fn load_runbook_for_command(
        &self,
        project_path: &Path,
        command: &str,
    ) -> Result<Runbook, RuntimeError> {
        let runbook_dir = project_path.join(".oj/runbooks");
        oj_runbook::find_runbook_by_command(&runbook_dir, command)
            .map_err(|e| RuntimeError::RunbookLoadError(e.to_string()))?
            .ok_or_else(|| RuntimeError::CommandNotFound(command.to_string()))
    }

    /// Re-read a supervisor's runbook from disk and return a `RunbookLoaded`
    /// event if the content changed. Returns `Ok(None)` when unchanged.
    pub(crate) fn refresh_runbook(
        &self,
        project_path: &Path,
        old_hash: &str,
        scoped_key: &str,
        find_runbook: impl FnOnce(&Path, &str) -> Result<Option<Runbook>, oj_runbook::FindError>,
        apply: impl FnOnce(&str, String, &Runbook),
    ) -> Result<Option<oj_core::Event>, RuntimeError> {
        let (_, bare_name) = oj_core::split_scoped_name(scoped_key);
        let runbook_dir = project_path.join(".oj/runbooks");
        let runbook = find_runbook(&runbook_dir, bare_name)
            .map_err(|e| RuntimeError::RunbookLoadError(e.to_string()))?
            .ok_or_else(|| {
                RuntimeError::RunbookLoadError(format!(
                    "no runbook found containing '{}'",
                    bare_name
                ))
            })?;

        let runbook_json = serde_json::to_value(&runbook)
            .map_err(|e| RuntimeError::RunbookLoadError(format!("failed to serialize: {}", e)))?;
        let runbook_hash = {
            let canonical = serde_json::to_string(&runbook_json).map_err(|e| {
                RuntimeError::RunbookLoadError(format!("failed to serialize: {}", e))
            })?;
            format!("{:x}", Sha256::digest(canonical.as_bytes()))
        };

        if old_hash == runbook_hash {
            return Ok(None);
        }

        tracing::info!(
            supervisor = scoped_key,
            old_hash = oj_core::short(old_hash, 12),
            new_hash = oj_core::short(&runbook_hash, 12),
            "runbook changed on disk, refreshing"
        );

        apply(bare_name, runbook_hash.clone(), &runbook);

        // Update in-process cache
        {
            let mut cache = self.runbook_cache.lock();
            cache.insert(runbook_hash.clone(), runbook);
        }

        Ok(Some(oj_core::Event::RunbookLoaded {
            hash: runbook_hash,
            version: 1,
            runbook: runbook_json,
        }))
    }

    pub(crate) fn refresh_worker_runbook(
        &self,
        scoped_key: &str,
    ) -> Result<Option<oj_core::Event>, RuntimeError> {
        let (project_path, old_hash) = {
            let guard = self.worker_states.lock();
            match guard.get(scoped_key) {
                Some(s) => (s.project_path.clone(), s.runbook_hash.clone()),
                None => return Ok(None),
            }
        };
        self.refresh_runbook(
            &project_path,
            &old_hash,
            scoped_key,
            oj_runbook::find_runbook_by_worker,
            |_, hash, _| {
                if let Some(s) = self.worker_states.lock().get_mut(scoped_key) {
                    s.runbook_hash = hash;
                }
            },
        )
    }

    pub(crate) fn refresh_cron_runbook(
        &self,
        scoped_key: &str,
    ) -> Result<Option<oj_core::Event>, RuntimeError> {
        let (project_path, old_hash) = {
            let guard = self.cron_states.lock();
            match guard.get(scoped_key) {
                Some(s) => (s.project_path.clone(), s.runbook_hash.clone()),
                None => return Ok(None),
            }
        };
        self.refresh_runbook(
            &project_path,
            &old_hash,
            scoped_key,
            oj_runbook::find_runbook_by_cron,
            |bare_name, hash, runbook| {
                if let Some(s) = self.cron_states.lock().get_mut(scoped_key) {
                    s.runbook_hash = hash;
                    s.concurrency =
                        runbook.get_cron(bare_name).and_then(|c| c.concurrency).unwrap_or(1);
                }
            },
        )
    }

    /// Load a runbook from disk by worker name, hash it, and cache it.
    ///
    /// Used as a fallback when `cached_runbook` fails (e.g. after daemon
    /// restart when the in-process cache is empty and the stored runbook
    /// can't be deserialized due to schema changes).
    ///
    /// Returns `(runbook, hash, RunbookLoaded event)`.
    pub(crate) fn load_runbook_from_disk(
        &self,
        project_path: &Path,
        worker_name: &str,
    ) -> Result<(Runbook, String, oj_core::Event), RuntimeError> {
        let runbook_dir = project_path.join(".oj/runbooks");
        let runbook = oj_runbook::find_runbook_by_worker(&runbook_dir, worker_name)
            .map_err(|e| RuntimeError::RunbookLoadError(e.to_string()))?
            .ok_or_else(|| {
                RuntimeError::RunbookLoadError(format!(
                    "no runbook found containing worker '{}'",
                    worker_name
                ))
            })?;

        let runbook_json = serde_json::to_value(&runbook)
            .map_err(|e| RuntimeError::RunbookLoadError(format!("failed to serialize: {}", e)))?;
        let runbook_hash = {
            let canonical = serde_json::to_string(&runbook_json).map_err(|e| {
                RuntimeError::RunbookLoadError(format!("failed to serialize: {}", e))
            })?;
            format!("{:x}", Sha256::digest(canonical.as_bytes()))
        };

        // Populate in-process cache
        {
            let mut cache = self.runbook_cache.lock();
            cache.insert(runbook_hash.clone(), runbook.clone());
        }

        let event = oj_core::Event::RunbookLoaded {
            hash: runbook_hash.clone(),
            version: 1,
            runbook: runbook_json,
        };

        Ok((runbook, runbook_hash, event))
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
