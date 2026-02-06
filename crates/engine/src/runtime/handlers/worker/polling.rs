// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue polling: wake, poll persisted/external queues, poll timer

use super::WorkerStatus;
use crate::error::RuntimeError;
use crate::runtime::Runtime;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{scoped_name, Clock, Effect, Event, TimerId};
use oj_runbook::QueueType;
use oj_storage::QueueItemStatus;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_worker_wake(
        &self,
        worker_name: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        tracing::info!(worker = worker_name, "worker wake");
        let mut result_events = Vec::new();

        // Log wake event
        {
            let workers = self.worker_states.lock();
            if let Some(state) = workers.get(worker_name) {
                let scoped = scoped_name(&state.namespace, worker_name);
                self.worker_logger.append(&scoped, "wake");
            }
        }

        // Refresh runbook from disk so edits after `oj worker start` are picked up
        if let Some(loaded_event) = self.refresh_worker_runbook(worker_name)? {
            result_events.push(loaded_event);
        }

        let (queue_type, queue_name, runbook_hash, project_root, worker_namespace, poll_interval) = {
            let workers = self.worker_states.lock();
            let state = match workers.get(worker_name) {
                Some(s) if s.status != WorkerStatus::Stopped => s,
                _ => {
                    tracing::warn!(worker = worker_name, "worker wake: not found or stopped");
                    return Ok(result_events);
                }
            };
            (
                state.queue_type,
                state.queue_name.clone(),
                state.runbook_hash.clone(),
                state.project_root.clone(),
                state.namespace.clone(),
                state.poll_interval.clone(),
            )
        };

        match queue_type {
            QueueType::External => {
                let runbook = self.cached_runbook(&runbook_hash)?;
                let queue_def = runbook.get_queue(&queue_name).ok_or_else(|| {
                    RuntimeError::WorkerNotFound(format!("queue '{}' not found", queue_name))
                })?;

                let poll_effect = Effect::PollQueue {
                    worker_name: worker_name.to_string(),
                    list_command: queue_def.list.clone().unwrap_or_default(),
                    cwd: project_root,
                };
                result_events.extend(self.executor.execute_all(vec![poll_effect]).await?);

                // Ensure poll timer is set so periodic polling continues.
                // This is the sole owner of the timer — both timer-fired and
                // direct wakes (WorkerWake, resize) converge here.
                if let Some(ref poll) = poll_interval {
                    let duration = crate::monitor::parse_duration(poll).map_err(|e| {
                        RuntimeError::InvalidFormat(format!(
                            "invalid poll interval '{}': {}",
                            poll, e
                        ))
                    })?;
                    let timer_id = TimerId::queue_poll(worker_name, &worker_namespace);
                    self.executor
                        .execute(Effect::SetTimer {
                            id: timer_id,
                            duration,
                        })
                        .await?;
                }
            }
            QueueType::Persisted => {
                result_events.extend(self.poll_persisted_queue(
                    worker_name,
                    &queue_name,
                    &worker_namespace,
                )?);
            }
        }

        Ok(result_events)
    }

    /// Read pending items from MaterializedState and synthesize a WorkerPollComplete event.
    pub(super) fn poll_persisted_queue(
        &self,
        worker_name: &str,
        queue_name: &str,
        namespace: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let key = scoped_name(namespace, queue_name);
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
            worker = worker_name,
            queue = queue_name,
            pending = items.len(),
            total,
            "polled persisted queue"
        );

        // Synthesize a WorkerPollComplete event to reuse the existing dispatch flow
        Ok(vec![Event::WorkerPollComplete {
            worker_name: worker_name.to_string(),
            items,
        }])
    }

    /// Handle a queue poll timer firing: wake the worker to re-poll and reschedule.
    pub(crate) async fn handle_queue_poll_timer(
        &self,
        rest: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Parse worker name from timer ID (after "queue-poll:" prefix)
        // Format: "worker_name" or "namespace/worker_name"
        let worker_name = rest.rsplit('/').next().unwrap_or(rest);

        tracing::debug!(worker = worker_name, "queue poll timer fired");

        // Wake handles polling and rescheduling the timer
        self.handle_worker_wake(worker_name).await
    }
}

#[cfg(test)]
#[path = "polling_tests.rs"]
mod tests;
