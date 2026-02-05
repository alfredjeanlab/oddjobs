// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker start/stop lifecycle handling

use super::{WorkerState, WorkerStatus};
use crate::error::RuntimeError;
use crate::runtime::Runtime;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{scoped_name, Clock, Effect, Event, JobId, TimerId};
use oj_runbook::QueueType;
use oj_storage::QueueItemStatus;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_worker_started(
        &self,
        worker_name: &str,
        project_root: &Path,
        runbook_hash: &str,
        namespace: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Load runbook to get worker definition
        let runbook = self.cached_runbook(runbook_hash)?;
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
            let scoped = scoped_name(namespace, worker_name);
            let active: HashSet<JobId> = state
                .workers
                .get(&scoped)
                .map(|w| w.active_job_ids.iter().map(JobId::new).collect())
                .unwrap_or_default();

            // Build job→item map from persisted job vars
            let item_map: HashMap<JobId, String> = active
                .iter()
                .filter_map(|pid| {
                    state
                        .jobs
                        .get(pid.as_str())
                        .and_then(|p| p.vars.get("item.id"))
                        .map(|item_id| (pid.clone(), item_id.clone()))
                })
                .collect();

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
            project_root: project_root.to_path_buf(),
            runbook_hash: runbook_hash.to_string(),
            queue_name: worker_def.source.queue.clone(),
            job_kind: worker_def.handler.job.clone(),
            concurrency: worker_def.concurrency,
            active_jobs: persisted_active,
            status: WorkerStatus::Running,
            queue_type,
            item_job_map: persisted_item_map,
            namespace: namespace.to_string(),
            poll_interval: poll_interval.clone(),
            pending_takes: 0,
            inflight_items: persisted_inflight,
        };

        {
            let mut workers = self.worker_states.lock();
            workers.insert(worker_name.to_string(), state);
        }

        let scoped = scoped_name(namespace, worker_name);
        self.worker_logger.append(
            &scoped,
            &format!(
                "started (queue={}, concurrency={})",
                worker_def.source.queue, worker_def.concurrency
            ),
        );

        // Reconcile: release queue items whose jobs are already terminal.
        // This handles the case where the daemon crashed after a job completed
        // but before the QueueCompleted/QueueFailed event was persisted.
        if queue_type == QueueType::Persisted {
            self.reconcile_queue_items(worker_name, namespace, &runbook)
                .await?;
        }

        // Trigger initial poll
        match queue_type {
            QueueType::External => {
                let list_command = queue_def.list.clone().unwrap_or_default();
                let events = self
                    .executor
                    .execute_all(vec![Effect::PollQueue {
                        worker_name: worker_name.to_string(),
                        list_command,
                        cwd: project_root.to_path_buf(),
                    }])
                    .await?;

                // Start periodic poll timer if configured
                if let Some(ref poll) = poll_interval {
                    let duration = crate::monitor::parse_duration(poll).map_err(|e| {
                        RuntimeError::InvalidFormat(format!(
                            "invalid poll interval '{}': {}",
                            poll, e
                        ))
                    })?;
                    let timer_id = TimerId::queue_poll(worker_name, namespace);
                    self.executor
                        .execute(Effect::SetTimer {
                            id: timer_id,
                            duration,
                        })
                        .await?;
                }

                Ok(events)
            }
            QueueType::Persisted => {
                self.poll_persisted_queue(worker_name, &worker_def.source.queue, namespace)
            }
        }
    }

    pub(crate) async fn handle_worker_stopped(
        &self,
        worker_name: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let namespace = {
            let mut workers = self.worker_states.lock();
            if let Some(state) = workers.get_mut(worker_name) {
                let scoped = scoped_name(&state.namespace, worker_name);
                self.worker_logger.append(&scoped, "stopped");
                state.status = WorkerStatus::Stopped;
                state.pending_takes = 0;
                state.inflight_items.clear();
                state.namespace.clone()
            } else {
                String::new()
            }
        };

        // Cancel poll timer if it was set (no-op if timer doesn't exist)
        let timer_id = TimerId::queue_poll(worker_name, &namespace);
        self.executor
            .execute(Effect::CancelTimer { id: timer_id })
            .await?;

        Ok(vec![])
    }

    pub(crate) async fn handle_worker_resized(
        &self,
        worker_name: &str,
        new_concurrency: u32,
        namespace: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let (old_concurrency, should_poll) = {
            let mut workers = self.worker_states.lock();
            match workers.get_mut(worker_name) {
                Some(state) if state.status == WorkerStatus::Running => {
                    let old = state.concurrency;
                    state.concurrency = new_concurrency;

                    // Check if we now have more slots available
                    let active = state.active_jobs.len() as u32 + state.pending_takes;
                    let had_capacity = old > active;
                    let has_capacity = new_concurrency > active;
                    let should_poll = !had_capacity && has_capacity;

                    (old, should_poll)
                }
                _ => return Ok(vec![]),
            }
        };

        // Log the resize
        let scoped = scoped_name(namespace, worker_name);
        self.worker_logger.append(
            &scoped,
            &format!(
                "resized concurrency {} → {}",
                old_concurrency, new_concurrency
            ),
        );

        // If we went from full to having capacity, trigger re-poll
        if should_poll {
            return self.handle_worker_wake(worker_name).await;
        }

        Ok(vec![])
    }

    /// Reconcile queue items after daemon recovery.
    ///
    /// Handles three cases:
    /// 1. Active jobs that already reached terminal state — calls
    ///    `check_worker_job_complete` to emit the missing queue events.
    /// 2. Active queue items with a running job not tracked by worker —
    ///    adds the job to worker's active list.
    /// 3. Active queue items with no corresponding job (pruned/lost) —
    ///    fails them with retry-or-dead logic.
    async fn reconcile_queue_items(
        &self,
        worker_name: &str,
        namespace: &str,
        runbook: &oj_runbook::Runbook,
    ) -> Result<(), RuntimeError> {
        // 1. Release queue items whose jobs are already terminal
        let active_pids: Vec<JobId> = {
            let workers = self.worker_states.lock();
            workers
                .get(worker_name)
                .map(|s| s.active_jobs.iter().cloned().collect())
                .unwrap_or_default()
        };
        let terminal_jobs: Vec<(JobId, String)> = self.lock_state(|state| {
            active_pids
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

        for (pid, terminal_step) in terminal_jobs {
            tracing::info!(
                worker = worker_name,
                job = pid.as_str(),
                step = terminal_step.as_str(),
                "reconciling terminal job for queue item"
            );
            let _ = self.check_worker_job_complete(&pid, &terminal_step).await;
        }

        // 2. Find and track active queue items with running jobs not in worker's active list
        let queue_name = {
            let workers = self.worker_states.lock();
            workers
                .get(worker_name)
                .map(|s| s.queue_name.clone())
                .unwrap_or_default()
        };
        let scoped_queue = scoped_name(namespace, &queue_name);
        let mapped_item_ids: HashSet<String> = {
            let workers = self.worker_states.lock();
            workers
                .get(worker_name)
                .map(|s| s.item_job_map.values().cloned().collect())
                .unwrap_or_default()
        };

        // Find Active queue items assigned to this worker but not in item_job_map,
        // then check if a corresponding job exists (by searching for item.id var match)
        let untracked_items: Vec<(String, JobId)> = self.lock_state(|state| {
            let active_items: Vec<String> = state
                .queue_items
                .get(&scoped_queue)
                .map(|items| {
                    items
                        .iter()
                        .filter(|i| {
                            i.status == QueueItemStatus::Active
                                && i.worker_name.as_deref() == Some(worker_name)
                                && !mapped_item_ids.contains(&i.id)
                        })
                        .map(|i| i.id.clone())
                        .collect()
                })
                .unwrap_or_default();

            // For each active item, look for a job with matching item.id
            active_items
                .into_iter()
                .filter_map(|item_id| {
                    state
                        .jobs
                        .iter()
                        .find(|(_, job)| {
                            job.vars.get("item.id") == Some(&item_id) && !job.is_terminal()
                        })
                        .map(|(job_id, _)| (item_id, JobId::new(job_id.clone())))
                })
                .collect()
        });

        // Add untracked jobs to worker's active list
        for (item_id, job_id) in untracked_items {
            tracing::info!(
                worker = worker_name,
                item_id = item_id.as_str(),
                job_id = job_id.as_str(),
                "reconciling untracked job for active queue item"
            );
            {
                let mut workers = self.worker_states.lock();
                if let Some(state) = workers.get_mut(worker_name) {
                    if !state.active_jobs.contains(&job_id) {
                        state.active_jobs.insert(job_id.clone());
                    }
                    state.item_job_map.insert(job_id.clone(), item_id.clone());
                }
            }
            // Emit WorkerItemDispatched to persist the tracking
            self.executor
                .execute_all(vec![Effect::Emit {
                    event: Event::WorkerItemDispatched {
                        worker_name: worker_name.to_string(),
                        item_id,
                        job_id,
                        namespace: namespace.to_string(),
                    },
                }])
                .await?;
        }

        // 3. Fail active queue items with no corresponding job
        // Re-fetch mapped_item_ids after adding untracked jobs
        let mapped_item_ids: HashSet<String> = {
            let workers = self.worker_states.lock();
            workers
                .get(worker_name)
                .map(|s| s.item_job_map.values().cloned().collect())
                .unwrap_or_default()
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
                                && i.worker_name.as_deref() == Some(worker_name)
                                && !mapped_item_ids.contains(&i.id)
                        })
                        .map(|i| i.id.clone())
                        .collect()
                })
                .unwrap_or_default()
        });

        for item_id in orphaned_items {
            tracing::info!(
                worker = worker_name,
                item_id = item_id.as_str(),
                "reconciling orphaned queue item (no job)"
            );

            self.executor
                .execute_all(vec![Effect::Emit {
                    event: Event::QueueFailed {
                        queue_name: queue_name.clone(),
                        item_id: item_id.clone(),
                        error: "job lost during daemon recovery".to_string(),
                        namespace: namespace.to_string(),
                    },
                }])
                .await?;

            // Apply retry-or-dead logic
            let failure_count = self.lock_state(|state| {
                state
                    .queue_items
                    .get(&scoped_queue)
                    .and_then(|items| items.iter().find(|i| i.id == item_id))
                    .map(|i| i.failure_count)
                    .unwrap_or(0)
            });

            let retry_config = runbook
                .get_queue(&queue_name)
                .and_then(|q| q.retry.as_ref());
            let max_attempts = retry_config.map(|r| r.attempts).unwrap_or(0);

            if max_attempts > 0 && failure_count < max_attempts {
                let cooldown_str = retry_config.map(|r| r.cooldown.as_str()).unwrap_or("0s");
                let duration =
                    crate::monitor::parse_duration(cooldown_str).unwrap_or(Duration::ZERO);
                let timer_id = TimerId::queue_retry(&scoped_queue, &item_id);
                self.executor
                    .execute(Effect::SetTimer {
                        id: timer_id,
                        duration,
                    })
                    .await?;
            } else {
                self.executor
                    .execute_all(vec![Effect::Emit {
                        event: Event::QueueItemDead {
                            queue_name: queue_name.clone(),
                            item_id,
                            namespace: namespace.to_string(),
                        },
                    }])
                    .await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
