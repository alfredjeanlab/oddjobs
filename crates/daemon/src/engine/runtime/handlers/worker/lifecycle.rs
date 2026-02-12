// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker start/stop lifecycle handling

use super::{WorkerState, WorkerStatus};
use crate::adapters::{AgentAdapter, NotifyAdapter};
use crate::engine::error::RuntimeError;
use crate::engine::runtime::Runtime;
use crate::storage::QueueItemStatus;
use oj_core::{scoped_name, split_scoped_name, Clock, Effect, Event, JobId, OwnerId, TimerId};
use oj_runbook::QueueType;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

impl<A, N, C> Runtime<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_worker_started(
        &self,
        worker_name: &str,
        project_path: &Path,
        runbook_hash: &str,
        project: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let worker_key = scoped_name(project, worker_name);
        let mut result_events = Vec::new();

        // Defense in depth: if this worker is already Running in memory, a second
        // WorkerStarted would replace the WorkerState and clear inflight_items /
        // pending_takes, allowing duplicate dispatches.  Delegate to wake instead
        // so in-flight tracking is preserved.
        let already_running = {
            let workers = self.worker_states.lock();
            workers.get(&worker_key).is_some_and(|s| s.status == WorkerStatus::Running)
        };
        if already_running {
            tracing::warn!(
                worker = worker_name,
                "duplicate WorkerStarted for running worker, delegating to wake"
            );
            return self.handle_worker_wake(&worker_key).await;
        }

        // Load runbook to get worker definition.
        // Try cached/state first; fall back to disk (handles daemon restart when
        // the in-process cache is empty and state deserialization fails).
        let (runbook, runbook_hash) = match self.cached_runbook(runbook_hash) {
            Ok(rb) => (rb, runbook_hash.to_string()),
            Err(cache_err) => {
                tracing::warn!(
                    worker = worker_name,
                    hash = runbook_hash,
                    error = %cache_err,
                    "cached runbook lookup failed, loading from disk"
                );
                let (rb, hash, loaded_event) =
                    self.load_runbook_from_disk(project_path, worker_name)?;
                result_events.push(loaded_event);
                (rb, hash)
            }
        };
        let worker_def = runbook
            .get_worker(worker_name)
            .ok_or_else(|| RuntimeError::WorkerNotFound(worker_name.to_string()))?;

        let queue_def = runbook.get_queue(&worker_def.source.queue).ok_or_else(|| {
            RuntimeError::WorkerNotFound(format!(
                "queue '{}' not found for worker '{}'",
                worker_def.source.queue, worker_name
            ))
        })?;

        let queue_type = queue_def.queue_type;

        // Restore active jobs from persisted state (survives daemon restart)
        let (persisted_active, persisted_item_map, persisted_inflight) = self.lock_state(|state| {
            let scoped = scoped_name(project, worker_name);
            let record = state.workers.get(&scoped);

            let active: HashSet<OwnerId> = record
                .map(|w| w.active.iter().map(|s| OwnerId::parse(s)).collect())
                .unwrap_or_default();

            // Restore owner→item map from persisted WorkerRecord
            let item_map: HashMap<OwnerId, String> = record
                .map(|w| {
                    w.owners
                        .iter()
                        .map(|(owner_str, item_id)| (OwnerId::parse(owner_str), item_id.clone()))
                        .collect()
                })
                .unwrap_or_default();

            // For external queues, restore inflight item IDs so overlapping
            // polls after restart don't re-dispatch already-active items.
            let inflight: HashSet<String> = if queue_type == QueueType::External {
                item_map.values().cloned().collect()
            } else {
                HashSet::new()
            };

            (active, item_map, inflight)
        });

        // Store worker state
        let poll_interval = queue_def.poll.clone();
        let state = WorkerState {
            project_path: project_path.to_path_buf(),
            runbook_hash: runbook_hash.to_string(),
            queue_name: worker_def.source.queue.clone(),
            job_kind: worker_def.run.job.clone(),
            concurrency: worker_def.concurrency,
            active: persisted_active,
            status: WorkerStatus::Running,
            queue_type,
            items: persisted_item_map,
            project: project.to_string(),
            poll_interval: poll_interval.clone(),
            pending_takes: 0,
            inflight_items: persisted_inflight,
        };

        {
            let mut workers = self.worker_states.lock();
            workers.insert(worker_key.clone(), state);
        }

        self.worker_logger.append(
            &worker_key,
            &format!(
                "started (queue={}, concurrency={})",
                worker_def.source.queue, worker_def.concurrency
            ),
        );

        // Reconcile: release active jobs that already reached terminal state.
        // This handles the case where the daemon crashed after a job completed
        // but before the worker slot was freed. Runs for all queue types.
        result_events.extend(self.reconcile_active_jobs(&worker_key).await?);

        // Reconcile persisted queue items: track untracked jobs and fail orphaned items.
        if queue_type == QueueType::Persisted {
            self.reconcile_queue_items(&worker_key, project, &runbook).await?;
        }

        // Trigger initial poll
        match queue_type {
            QueueType::External => {
                let list_command = queue_def.list.clone().unwrap_or_default();
                result_events.extend(
                    self.executor
                        .execute_all(vec![Effect::PollQueue {
                            worker_name: worker_name.to_string(),
                            project: project.to_string(),
                            list_command,
                            cwd: project_path.to_path_buf(),
                        }])
                        .await?,
                );

                // Start periodic poll timer if configured
                if let Some(ref poll) = poll_interval {
                    let duration = crate::engine::monitor::parse_duration(poll).map_err(|e| {
                        RuntimeError::InvalidFormat(format!(
                            "invalid poll interval '{}': {}",
                            poll, e
                        ))
                    })?;
                    let timer_id = TimerId::queue_poll(worker_name, project);
                    self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;
                }

                Ok(result_events)
            }
            QueueType::Persisted => {
                result_events.extend(self.poll_persisted_queue(
                    &worker_key,
                    &worker_def.source.queue,
                    project,
                )?);
                Ok(result_events)
            }
        }
    }

    pub(crate) async fn handle_worker_stopped(
        &self,
        worker_key: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let (bare_name, project) = {
            let mut workers = self.worker_states.lock();
            if let Some(state) = workers.get_mut(worker_key) {
                self.worker_logger.append(worker_key, "stopped");
                state.status = WorkerStatus::Stopped;
                state.pending_takes = 0;
                state.inflight_items.clear();
                let (_, bare) = split_scoped_name(worker_key);
                (bare.to_string(), state.project.clone())
            } else {
                let (_, bare) = split_scoped_name(worker_key);
                (bare.to_string(), String::new())
            }
        };

        // Cancel poll timer if it was set (no-op if timer doesn't exist)
        let timer_id = TimerId::queue_poll(&bare_name, &project);
        self.executor.execute(Effect::CancelTimer { id: timer_id }).await?;

        Ok(vec![])
    }

    pub(crate) async fn handle_worker_resized(
        &self,
        worker_name: &str,
        new_concurrency: u32,
        project: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let worker_key = scoped_name(project, worker_name);
        let (old_concurrency, should_poll) = {
            let mut workers = self.worker_states.lock();
            match workers.get_mut(&worker_key) {
                Some(state) if state.status == WorkerStatus::Running => {
                    let old = state.concurrency;
                    state.concurrency = new_concurrency;

                    // Check if we now have more slots available
                    let active = state.active.len() as u32 + state.pending_takes;
                    let had_capacity = old > active;
                    let has_capacity = new_concurrency > active;
                    let should_poll = !had_capacity && has_capacity;

                    (old, should_poll)
                }
                _ => return Ok(vec![]),
            }
        };

        // Log the resize
        self.worker_logger.append(
            &worker_key,
            &format!("resized concurrency {} → {}", old_concurrency, new_concurrency),
        );

        // If we went from full to having capacity, trigger re-poll
        if should_poll {
            return self.handle_worker_wake(&worker_key).await;
        }

        Ok(vec![])
    }

    /// Reconcile active jobs after daemon recovery.
    ///
    /// Checks if any jobs in the worker's active set have already reached
    /// terminal state, and calls `check_worker_job_complete` to emit the
    /// missing queue events and free the worker slot.
    ///
    /// Runs for ALL queue types (external and persisted).
    async fn reconcile_active_jobs(&self, worker_key: &str) -> Result<Vec<Event>, RuntimeError> {
        let active_owners: Vec<OwnerId> = {
            let workers = self.worker_states.lock();
            workers.get(worker_key).map(|s| s.active.iter().cloned().collect()).unwrap_or_default()
        };
        // For now, workers only dispatch jobs. Extract JobIds for reconciliation.
        let active_job_ids: Vec<JobId> = active_owners
            .into_iter()
            .filter_map(|o| match o {
                OwnerId::Job(id) => Some(id),
                _ => None,
            })
            .collect();
        let terminal_jobs: Vec<(JobId, String)> = self.lock_state(|state| {
            active_job_ids
                .iter()
                .filter_map(|pid| {
                    state
                        .jobs
                        .get(pid.as_str())
                        .filter(|p| p.is_terminal())
                        .map(|p| (pid.clone(), p.step.clone()))
                })
                .collect()
        });

        let mut events = Vec::new();
        for (pid, terminal_step) in terminal_jobs {
            tracing::info!(
                worker = worker_key,
                job = pid.as_str(),
                step = terminal_step.as_str(),
                "reconciling terminal job for worker slot"
            );
            match self.check_worker_job_complete(&pid, &terminal_step).await {
                Ok(evts) => events.extend(evts),
                Err(e) => {
                    tracing::warn!(
                        worker = worker_key,
                        job = pid.as_str(),
                        error = %e,
                        "failed to reconcile terminal job"
                    );
                }
            }
        }

        Ok(events)
    }

    /// Reconcile queue items after daemon recovery.
    ///
    /// Handles two cases:
    /// 1. Active queue items with a running job not tracked by worker —
    ///    adds the job to worker's active list.
    /// 2. Active queue items with no corresponding job (pruned/lost) —
    ///    fails them with retry-or-dead logic.
    async fn reconcile_queue_items(
        &self,
        worker_key: &str,
        project: &str,
        runbook: &oj_runbook::Runbook,
    ) -> Result<(), RuntimeError> {
        let (_, bare_name) = split_scoped_name(worker_key);
        // 1. Find and track active queue items with running jobs not in worker's active list
        let queue_name = {
            let workers = self.worker_states.lock();
            workers.get(worker_key).map(|s| s.queue_name.clone()).unwrap_or_default()
        };
        let scoped_queue = scoped_name(project, &queue_name);
        let mapped_item_ids: HashSet<String> = {
            let workers = self.worker_states.lock();
            workers.get(worker_key).map(|s| s.items.values().cloned().collect()).unwrap_or_default()
        };

        // Recover item→job mappings from the materialized WorkerRecord.
        // This handles cases where the runtime item_owners was lost (e.g.,
        // daemon restart) but WorkerDispatched events exist in the WAL.
        let untracked_items: Vec<(String, OwnerId)> = self.lock_state(|state| {
            let scoped = scoped_name(project, worker_key);
            state
                .workers
                .get(&scoped)
                .map(|record| {
                    record
                        .owners
                        .iter()
                        .filter(|(owner_str, item_id)| {
                            !mapped_item_ids.contains(item_id.as_str())
                                && state
                                    .jobs
                                    .get(owner_str.strip_prefix("job:").unwrap_or(owner_str))
                                    .is_some_and(|j| !j.is_terminal())
                        })
                        .map(|(owner_str, item_id)| (item_id.clone(), OwnerId::parse(owner_str)))
                        .collect()
                })
                .unwrap_or_default()
        });

        // Add untracked items to worker's active list
        for (item_id, owner) in untracked_items {
            tracing::info!(
                worker = worker_key,
                item_id = item_id.as_str(),
                owner = %owner,
                "reconciling untracked dispatched item"
            );
            let mut workers = self.worker_states.lock();
            if let Some(state) = workers.get_mut(worker_key) {
                if !state.active.contains(&owner) {
                    state.active.insert(owner.clone());
                }
                state.items.insert(owner, item_id);
            }
        }

        // 2. Fail active queue items with no corresponding job
        // Re-fetch mapped_item_ids after adding untracked jobs
        let mapped_item_ids: HashSet<String> = {
            let workers = self.worker_states.lock();
            workers.get(worker_key).map(|s| s.items.values().cloned().collect()).unwrap_or_default()
        };

        let orphaned_items: Vec<String> = self.lock_state(|state| {
            state
                .queue_items
                .get(&scoped_queue)
                .map(|items| {
                    items
                        .iter()
                        .filter(|i| {
                            i.status == QueueItemStatus::Active
                                && i.worker.as_deref() == Some(bare_name)
                                && !mapped_item_ids.contains(&i.id)
                        })
                        .map(|i| i.id.clone())
                        .collect()
                })
                .unwrap_or_default()
        });

        for item_id in orphaned_items {
            tracing::info!(
                worker = worker_key,
                item_id = item_id.as_str(),
                "reconciling orphaned queue item (no job)"
            );

            self.executor
                .execute_all(vec![Effect::Emit {
                    event: Event::QueueFailed {
                        queue: queue_name.clone(),
                        item_id: item_id.clone(),
                        error: "job lost during daemon recovery".to_string(),
                        project: project.to_string(),
                    },
                }])
                .await?;

            // Apply retry-or-dead logic
            let failures = self.lock_state(|state| {
                state
                    .queue_items
                    .get(&scoped_queue)
                    .and_then(|items| items.iter().find(|i| i.id == item_id))
                    .map(|i| i.failures)
                    .unwrap_or(0)
            });

            let retry_config = runbook.get_queue(&queue_name).and_then(|q| q.retry.as_ref());
            let max_attempts = retry_config.map(|r| r.attempts).unwrap_or(0);

            if max_attempts > 0 && failures < max_attempts {
                let cooldown_str = retry_config.map(|r| r.cooldown.as_str()).unwrap_or("0s");
                let duration =
                    crate::engine::monitor::parse_duration(cooldown_str).unwrap_or(Duration::ZERO);
                let timer_id = TimerId::queue_retry(&scoped_queue, &item_id);
                self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;
            } else {
                self.executor
                    .execute_all(vec![Effect::Emit {
                        event: Event::QueueDead {
                            queue: queue_name.clone(),
                            item_id,
                            project: project.to_string(),
                        },
                    }])
                    .await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod reconcile_tests;
