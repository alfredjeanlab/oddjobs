// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job completion → queue item status updates

use super::WorkerStatus;
use crate::error::RuntimeError;
use crate::runtime::Runtime;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{scoped_name, Clock, Effect, Event, JobId, TimerId};
use oj_runbook::QueueType;
use std::collections::HashMap;
use std::time::Duration;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Check if a completed job belongs to a worker and trigger re-poll if so.
    /// For persisted queues, also emits queue:completed or queue:failed events.
    pub(crate) async fn check_worker_job_complete(
        &self,
        job_id: &JobId,
        terminal_step: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Find which worker (if any) owns this job
        let worker_info = {
            let mut workers = self.worker_states.lock();
            let mut found = None;
            for (name, state) in workers.iter_mut() {
                if state.active_jobs.remove(job_id) {
                    let item_id = state.item_job_map.remove(job_id);
                    // Remove from inflight set so the item can be re-queued
                    if let Some(ref id) = item_id {
                        state.inflight_items.remove(id);
                    }
                    found = Some((
                        name.clone(),
                        state.runbook_hash.clone(),
                        state.queue_name.clone(),
                        state.project_root.clone(),
                        state.queue_type,
                        item_id,
                        state.namespace.clone(),
                    ));
                    break;
                }
            }
            found
        };

        let mut result_events = Vec::new();

        if let Some((
            worker_name,
            _old_runbook_hash,
            queue_name,
            project_root,
            queue_type,
            item_id,
            worker_namespace,
        )) = worker_info
        {
            // Retrieve and clear stored item data for report interpolation
            let item_data = {
                let mut workers = self.worker_states.lock();
                workers
                    .get_mut(&worker_name)
                    .and_then(|s| item_id.as_ref().and_then(|id| s.item_data.remove(id)))
            };
            // Log job completion
            {
                let workers = self.worker_states.lock();
                let active = workers
                    .get(&worker_name)
                    .map(|s| s.active_jobs.len())
                    .unwrap_or(0);
                let concurrency = workers
                    .get(&worker_name)
                    .map(|s| s.concurrency)
                    .unwrap_or(0);
                let scoped = scoped_name(&worker_namespace, &worker_name);
                self.worker_logger.append(
                    &scoped,
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
            if let Some(loaded_event) = self.refresh_worker_runbook(&worker_name)? {
                result_events.push(loaded_event);
            }
            let runbook_hash = {
                let workers = self.worker_states.lock();
                workers
                    .get(&worker_name)
                    .map(|s| s.runbook_hash.clone())
                    .unwrap_or(_old_runbook_hash)
            };

            // For external queues with report config, execute report command
            if queue_type == QueueType::External {
                let runbook = self.cached_runbook(&runbook_hash)?;
                if let Some(queue_def) = runbook.get_queue(&queue_name) {
                    if let Some(ref report) = queue_def.report {
                        let is_completion = terminal_step == "done";

                        // Build interpolation vars from item data
                        let mut vars: HashMap<String, String> = HashMap::new();
                        if let Some(ref data) = item_data {
                            if let Some(obj) = data.as_object() {
                                for (key, value) in obj {
                                    let v = if let Some(s) = value.as_str() {
                                        s.to_string()
                                    } else {
                                        value.to_string()
                                    };
                                    vars.insert(format!("item.{}", key), v);
                                }
                            }
                        }
                        // Add error variable for on_fail
                        if !is_completion {
                            vars.insert(
                                "error".to_string(),
                                format!("job reached '{}'", terminal_step),
                            );
                        }

                        // Get the appropriate report command
                        let report_command = if is_completion {
                            report.on_done.as_ref()
                        } else {
                            report.on_fail.as_ref()
                        };

                        // Execute report command if configured
                        if let Some(cmd_template) = report_command {
                            let command = oj_runbook::interpolate_shell(cmd_template, &vars);
                            let item_id_str = item_id.clone().unwrap_or_default();
                            self.executor
                                .execute(Effect::ReportQueueItem {
                                    worker_name: worker_name.clone(),
                                    command,
                                    cwd: project_root.clone(),
                                    item_id: item_id_str,
                                    is_completion,
                                })
                                .await?;
                        }

                        // Update tracking counts
                        {
                            let mut workers = self.worker_states.lock();
                            if let Some(state) = workers.get_mut(&worker_name) {
                                if is_completion && state.track_completed {
                                    state.completed_count += 1;
                                } else if !is_completion && state.track_failed {
                                    state.failed_count += 1;
                                }
                            }
                        }
                    }
                }
            }

            // For persisted queues, emit queue completion/failure event
            if queue_type == QueueType::Persisted {
                if let Some(ref item_id) = item_id {
                    let queue_event = if terminal_step == "done" {
                        Event::QueueCompleted {
                            queue_name: queue_name.clone(),
                            item_id: item_id.clone(),
                            namespace: worker_namespace.clone(),
                        }
                    } else {
                        Event::QueueFailed {
                            queue_name: queue_name.clone(),
                            item_id: item_id.clone(),
                            error: format!("job reached '{}'", terminal_step),
                            namespace: worker_namespace.clone(),
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

                        // Read failure_count from state (QueueFailed already incremented it)
                        let failure_count = self.lock_state(|state| {
                            state
                                .queue_items
                                .get(&scoped_queue)
                                .and_then(|items| {
                                    items
                                        .iter()
                                        .find(|i| i.id == *item_id)
                                        .map(|i| i.failure_count)
                                })
                                .unwrap_or(0)
                        });

                        // Look up retry config from the runbook
                        let runbook = self.cached_runbook(&runbook_hash)?;
                        let retry_config = runbook
                            .get_queue(&queue_name)
                            .and_then(|q| q.retry.as_ref());

                        let max_attempts = retry_config.map(|r| r.attempts).unwrap_or(0);

                        if max_attempts > 0 && failure_count < max_attempts {
                            // Schedule retry after cooldown
                            let cooldown_str =
                                retry_config.map(|r| r.cooldown.as_str()).unwrap_or("0s");
                            let duration = crate::monitor::parse_duration(cooldown_str)
                                .unwrap_or(Duration::ZERO);
                            let timer_id = TimerId::queue_retry(&scoped_queue, item_id);
                            self.executor
                                .execute(Effect::SetTimer {
                                    id: timer_id,
                                    duration,
                                })
                                .await?;
                        } else {
                            // Mark as dead
                            result_events.extend(
                                self.executor
                                    .execute_all(vec![Effect::Emit {
                                        event: Event::QueueItemDead {
                                            queue_name: queue_name.clone(),
                                            item_id: item_id.clone(),
                                            namespace: worker_namespace.clone(),
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
                    .get(&worker_name)
                    .map(|s| {
                        s.status == WorkerStatus::Running
                            && (s.active_jobs.len() as u32) < s.concurrency
                    })
                    .unwrap_or(false)
            };

            if should_poll {
                match queue_type {
                    QueueType::External => {
                        let runbook = self.cached_runbook(&runbook_hash)?;
                        if let Some(queue_def) = runbook.get_queue(&queue_name) {
                            result_events.extend(
                                self.executor
                                    .execute_all(vec![Effect::PollQueue {
                                        worker_name,
                                        list_command: queue_def.list.clone().unwrap_or_default(),
                                        cwd: project_root,
                                    }])
                                    .await?,
                            );
                        }
                    }
                    QueueType::Persisted => {
                        result_events.extend(self.poll_persisted_queue(
                            &worker_name,
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
