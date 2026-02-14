// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job completion → queue item status updates

use super::WorkerStatus;
use crate::engine::error::RuntimeError;
use crate::engine::runtime::Runtime;
use oj_core::{scoped_name, split_scoped_name, Clock, Effect, Event, JobId, OwnerId, TimerId};
use oj_runbook::QueueType;
use std::time::Duration;

impl<C: Clock> Runtime<C> {
    /// Check if a completed job belongs to a worker and trigger re-poll if so.
    /// For persisted queues, also emits queue:completed or queue:failed events.
    pub(crate) async fn check_worker_job_complete(
        &self,
        job_id: &JobId,
        terminal_step: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Find which worker (if any) owns this job
        let worker_info = {
            let owner: OwnerId = (*job_id).into();
            let mut workers = self.worker_states.lock();
            let mut found = None;
            for (name, state) in workers.iter_mut() {
                if state.active.remove(&owner) {
                    let item_id = state.items.remove(&owner);
                    // Remove from inflight set so the item can be re-queued
                    if let Some(ref id) = item_id {
                        state.inflight_items.remove(id);
                    }
                    found = Some((
                        name.clone(),
                        state.runbook_hash.clone(),
                        state.queue_name.clone(),
                        state.project_path.clone(),
                        state.queue_type,
                        item_id,
                        state.project.clone(),
                    ));
                    break;
                }
            }
            found
        };

        let mut result_events = Vec::new();

        if let Some((
            worker_key,
            _old_runbook_hash,
            queue_name,
            project_path,
            queue_type,
            item_id,
            worker_namespace,
        )) = worker_info
        {
            // Log job completion
            {
                let workers = self.worker_states.lock();
                let active = workers.get(&worker_key).map(|s| s.active.len()).unwrap_or(0);
                let concurrency = workers.get(&worker_key).map(|s| s.concurrency).unwrap_or(0);
                self.worker_logger.append(
                    &worker_key,
                    &format!(
                        "job {} completed (step={}), active={}/{}",
                        job_id.as_str(),
                        terminal_step,
                        active,
                        concurrency,
                    ),
                );
            }

            // Refresh runbook from disk so edits after `oj worker start` are picked up
            if let Some(loaded_event) = self.refresh_worker_runbook(&worker_key)? {
                result_events.push(loaded_event);
            }
            let runbook_hash = {
                let workers = self.worker_states.lock();
                workers
                    .get(&worker_key)
                    .map(|s| s.runbook_hash.clone())
                    .unwrap_or(_old_runbook_hash)
            };

            // For persisted queues, emit queue completion/failure event.
            // Skip for suspended jobs — the queue item stays Active so it can
            // be retried when the job is resumed.
            if queue_type == QueueType::Persisted && terminal_step != "suspended" {
                if let Some(ref item_id) = item_id {
                    let queue_event = if terminal_step == "done" {
                        Event::QueueCompleted {
                            queue: queue_name.clone(),
                            item_id: item_id.clone(),
                            project: worker_namespace.clone(),
                        }
                    } else {
                        Event::QueueFailed {
                            queue: queue_name.clone(),
                            item_id: item_id.clone(),
                            error: format!("job reached '{}'", terminal_step),
                            project: worker_namespace.clone(),
                        }
                    };
                    result_events.extend(
                        self.executor
                            .execute_all(vec![Effect::Emit { event: queue_event }])
                            .await?,
                    );

                    // Retry-or-dead logic: after QueueFailed is applied, check retry config
                    if terminal_step != "done" {
                        let scoped_queue = scoped_name(&worker_namespace, &queue_name);

                        // Read failures from state (QueueFailed already incremented it)
                        let failures = self.lock_state(|state| {
                            state
                                .queue_items
                                .get(&scoped_queue)
                                .and_then(|items| {
                                    items.iter().find(|i| i.id == *item_id).map(|i| i.failures)
                                })
                                .unwrap_or(0)
                        });

                        // Look up retry config from the runbook
                        let runbook = self.cached_runbook(&runbook_hash)?;
                        let retry_config =
                            runbook.get_queue(&queue_name).and_then(|q| q.retry.as_ref());

                        let max_attempts = retry_config.map(|r| r.attempts).unwrap_or(0);

                        if max_attempts > 0 && failures < max_attempts {
                            // Schedule retry after cooldown
                            let cooldown_str =
                                retry_config.map(|r| r.cooldown.as_str()).unwrap_or("0s");
                            let duration = crate::engine::monitor::parse_duration(cooldown_str)
                                .unwrap_or(Duration::ZERO);
                            let timer_id = TimerId::queue_retry(&scoped_queue, item_id);
                            self.executor
                                .execute(Effect::SetTimer { id: timer_id, duration })
                                .await?;
                        } else {
                            // Mark as dead
                            result_events.extend(
                                self.executor
                                    .execute_all(vec![Effect::Emit {
                                        event: Event::QueueDead {
                                            queue: queue_name.clone(),
                                            item_id: item_id.clone(),
                                            project: worker_namespace.clone(),
                                        },
                                    }])
                                    .await?,
                            );
                        }
                    }
                }
            }

            // Check if worker is still running and has capacity
            let should_poll = {
                let workers = self.worker_states.lock();
                workers
                    .get(&worker_key)
                    .map(|s| {
                        s.status == WorkerStatus::Running && (s.active.len() as u32) < s.concurrency
                    })
                    .unwrap_or(false)
            };

            if should_poll {
                let (_, bare_name) = split_scoped_name(&worker_key);
                match queue_type {
                    QueueType::External => {
                        let runbook = self.cached_runbook(&runbook_hash)?;
                        if let Some(queue_def) = runbook.get_queue(&queue_name) {
                            result_events.extend(
                                self.executor
                                    .execute_all(vec![Effect::PollQueue {
                                        worker_name: bare_name.to_string(),
                                        project: worker_namespace,
                                        list_command: queue_def.list.clone().unwrap_or_default(),
                                        cwd: project_path,
                                    }])
                                    .await?,
                            );
                        }
                    }
                    QueueType::Persisted => {
                        result_events.extend(self.poll_persisted_queue(
                            &worker_key,
                            &queue_name,
                            &worker_namespace,
                        )?);
                    }
                }
            }
        }

        Ok(result_events)
    }
}
