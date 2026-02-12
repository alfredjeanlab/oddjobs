// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue polling: wake, poll persisted/external queues, poll timer

use super::WorkerStatus;
use crate::error::RuntimeError;
use crate::runtime::Runtime;
use oj_adapters::{AgentAdapter, NotifyAdapter};
use oj_core::{scoped_name, split_scoped_name, Clock, Effect, Event, TimerId};
use oj_runbook::QueueType;
use oj_storage::{QueueItemStatus, QueuePollMeta};

impl<A, N, C> Runtime<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_worker_wake(
        &self,
        worker_key: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        tracing::info!(worker = worker_key, "worker wake");
        let mut result_events = Vec::new();

        // Log wake event
        {
            let workers = self.worker_states.lock();
            if workers.get(worker_key).is_some() {
                self.worker_logger.append(worker_key, "wake");
            }
        }

        // Refresh runbook from disk so edits after `oj worker start` are picked up
        if let Some(loaded_event) = self.refresh_worker_runbook(worker_key)? {
            result_events.push(loaded_event);
        }

        let (_, bare_name) = split_scoped_name(worker_key);

        let (queue_type, queue_name, runbook_hash, project_path, worker_namespace, poll_interval) = {
            let workers = self.worker_states.lock();
            let state = match workers.get(worker_key) {
                Some(s) if s.status != WorkerStatus::Stopped => s,
                _ => {
                    tracing::warn!(worker = worker_key, "worker wake: not found or stopped");
                    return Ok(result_events);
                }
            };
            (
                state.queue_type,
                state.queue_name.clone(),
                state.runbook_hash.clone(),
                state.project_path.clone(),
                state.project.clone(),
                state.poll_interval.clone(),
            )
        };

        match queue_type {
            QueueType::External => {
                // Re-arm poll timer FIRST so periodic polling survives even if
                // this particular poll attempt fails (e.g., runbook parse error).
                // The timer was already removed from the scheduler when it fired,
                // so we must re-arm before any fallible operations.
                if let Some(ref poll) = poll_interval {
                    let duration = crate::monitor::parse_duration(poll).map_err(|e| {
                        RuntimeError::InvalidFormat(format!(
                            "invalid poll interval '{}': {}",
                            poll, e
                        ))
                    })?;
                    let timer_id = TimerId::queue_poll(bare_name, &worker_namespace);
                    self.executor.execute(Effect::SetTimer { id: timer_id, duration }).await?;
                }

                let runbook = self.cached_runbook(&runbook_hash)?;
                let queue_def = runbook.get_queue(&queue_name).ok_or_else(|| {
                    RuntimeError::WorkerNotFound(format!("queue '{}' not found", queue_name))
                })?;

                let poll_effect = Effect::PollQueue {
                    worker_name: bare_name.to_string(),
                    project: worker_namespace.clone(),
                    list_command: queue_def.list.clone().unwrap_or_default(),
                    cwd: project_path,
                };
                result_events.extend(self.executor.execute_all(vec![poll_effect]).await?);
            }
            QueueType::Persisted => {
                result_events.extend(self.poll_persisted_queue(
                    worker_key,
                    &queue_name,
                    &worker_namespace,
                )?);
            }
        }

        Ok(result_events)
    }

    /// Read pending items from MaterializedState and synthesize a WorkerPolled event.
    pub(super) fn poll_persisted_queue(
        &self,
        worker_key: &str,
        queue_name: &str,
        project: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let key = scoped_name(project, queue_name);
        let (total, items): (usize, Vec<serde_json::Value>) = self.lock_state(|state| match state
            .queue_items
            .get(&key)
        {
            Some(queue_items) => {
                let total = queue_items.len();
                let pending: Vec<_> = queue_items
                    .iter()
                    .filter(|item| item.status == QueueItemStatus::Pending)
                    .map(|item| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("id".to_string(), serde_json::Value::String(item.id.clone()));
                        for (k, v) in &item.data {
                            obj.insert(k.clone(), serde_json::Value::String(v.clone()));
                        }
                        serde_json::Value::Object(obj)
                    })
                    .collect();
                (total, pending)
            }
            None => (0, Vec::new()),
        });

        tracing::info!(
            worker = worker_key,
            queue = queue_name,
            pending = items.len(),
            total,
            "polled persisted queue"
        );

        self.lock_state_mut(|s| {
            s.poll_meta.insert(
                key.clone(),
                QueuePollMeta {
                    last_item_count: total,
                    last_polled_at_ms: self.executor.clock().epoch_ms(),
                },
            )
        });

        let (_, bare_name) = split_scoped_name(worker_key);
        // Synthesize a WorkerPolled event to reuse the existing dispatch flow
        Ok(vec![Event::WorkerPolled {
            worker: bare_name.to_string(),
            project: project.to_string(),
            items,
        }])
    }

    /// Handle a queue poll timer firing: wake the worker to re-poll and reschedule.
    pub(crate) async fn handle_queue_poll_timer(
        &self,
        rest: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Wake handles polling and rescheduling the timer
        self.handle_worker_wake(rest).await
    }
}
