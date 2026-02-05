// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue item dispatch: take items from queue and create jobs

use super::WorkerStatus;
use crate::error::RuntimeError;
use crate::runtime::handlers::CreateJobParams;
use crate::runtime::Runtime;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{scoped_name, Clock, Effect, Event, IdGen, JobId, UuidIdGen};
use oj_runbook::QueueType;
use oj_storage::QueueItemStatus;
use std::collections::HashMap;

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_worker_poll_complete(
        &self,
        worker_name: &str,
        items: &[serde_json::Value],
    ) -> Result<Vec<Event>, RuntimeError> {
        let mut result_events = Vec::new();

        // Refresh runbook from disk so edits after `oj worker start` are picked up
        if let Some(loaded_event) = self.refresh_worker_runbook(worker_name)? {
            result_events.push(loaded_event);
        }

        let (queue_type, take_template, cwd, available_slots, queue_name, worker_namespace) = {
            let mut workers = self.worker_states.lock();
            let state = match workers.get_mut(worker_name) {
                Some(s) if s.status != WorkerStatus::Stopped => s,
                _ => return Ok(result_events),
            };

            let active = state.active_jobs.len() as u32 + state.pending_takes;
            let available = state.concurrency.saturating_sub(active);
            if available == 0 || items.is_empty() {
                let scoped = scoped_name(&state.namespace, worker_name);
                self.worker_logger.append(
                    &scoped,
                    &format!("idle (active={}/{})", active, state.concurrency),
                );
                state.status = WorkerStatus::Running;
                return Ok(result_events);
            }

            let queue_type = state.queue_type;

            let runbook = self.cached_runbook(&state.runbook_hash)?;
            let queue_def = runbook.get_queue(&state.queue_name).ok_or_else(|| {
                RuntimeError::WorkerNotFound(format!("queue '{}' not found", state.queue_name))
            })?;

            state.status = WorkerStatus::Running;

            (
                queue_type,
                queue_def.take.clone(),
                state.project_root.clone(),
                available as usize,
                state.queue_name.clone(),
                state.namespace.clone(),
            )
        };

        let mut dispatched_count = 0;
        for item in items.iter() {
            if dispatched_count >= available_slots {
                break;
            }

            let item_id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            match queue_type {
                QueueType::External => {
                    // Guard against overlapping polls: skip items that already
                    // have an in-flight take command or an active job.
                    {
                        let workers = self.worker_states.lock();
                        if let Some(state) = workers.get(worker_name) {
                            if state.inflight_items.contains(&item_id) {
                                continue;
                            }
                        }
                    }

                    // Interpolate take command with item fields
                    let mut vars = HashMap::new();
                    if let Some(obj) = item.as_object() {
                        for (key, value) in obj {
                            let v = if let Some(s) = value.as_str() {
                                s.to_string()
                            } else {
                                value.to_string()
                            };
                            vars.insert(format!("item.{}", key), v);
                        }
                    }
                    let take_command = oj_runbook::interpolate_shell(
                        &take_template.clone().unwrap_or_default(),
                        &vars,
                    );

                    // Reserve concurrency slot and mark item as in-flight
                    // before firing the take command. The slot and inflight
                    // entry are released in handle_worker_take_complete.
                    {
                        let mut workers = self.worker_states.lock();
                        if let Some(state) = workers.get_mut(worker_name) {
                            state.pending_takes += 1;
                            state.inflight_items.insert(item_id.clone());
                        }
                    }

                    // Fire take command as background task. Job creation is
                    // deferred to handle_worker_take_complete when the command
                    // succeeds.
                    self.executor
                        .execute(Effect::TakeQueueItem {
                            worker_name: worker_name.to_string(),
                            take_command,
                            cwd: cwd.clone(),
                            item_id,
                            item: item.clone(),
                        })
                        .await?;
                    dispatched_count += 1;
                }
                QueueType::Persisted => {
                    // Guard against stale WorkerPollComplete events: if multiple
                    // polls run before any dispatches are processed, their payloads
                    // overlap. Skip items that are no longer Pending to avoid
                    // creating duplicate jobs for the same queue item.
                    let scoped_queue = scoped_name(&worker_namespace, &queue_name);
                    let still_pending = self.lock_state(|state| {
                        state
                            .queue_items
                            .get(&scoped_queue)
                            .and_then(|items| items.iter().find(|i| i.id == item_id))
                            .map(|i| i.status == QueueItemStatus::Pending)
                            .unwrap_or(false)
                    });
                    if !still_pending {
                        continue;
                    }

                    // Emit queue:taken event via Effect::Emit
                    result_events.extend(
                        self.executor
                            .execute_all(vec![Effect::Emit {
                                event: Event::QueueTaken {
                                    queue_name: queue_name.clone(),
                                    item_id: item_id.clone(),
                                    worker_name: worker_name.to_string(),
                                    namespace: worker_namespace.clone(),
                                },
                            }])
                            .await?,
                    );

                    // Dispatch job immediately for persisted queues
                    result_events.extend(self.dispatch_queue_item(worker_name, item).await?);
                    dispatched_count += 1;
                }
            }
        }

        Ok(result_events)
    }

    /// Handle a completed take command for an external queue item.
    ///
    /// On success (exit_code == 0), creates a job for the item.
    /// On failure, logs the error and skips the item.
    pub(crate) async fn handle_worker_take_complete(
        &self,
        worker_name: &str,
        item_id: &str,
        item: &serde_json::Value,
        exit_code: i32,
        stderr: Option<&str>,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Release the pending-take slot reserved by handle_worker_poll_complete
        {
            let mut workers = self.worker_states.lock();
            if let Some(state) = workers.get_mut(worker_name) {
                state.pending_takes = state.pending_takes.saturating_sub(1);
            }
        }

        if exit_code != 0 {
            let err_msg = stderr.unwrap_or("unknown error");
            tracing::warn!(
                worker = worker_name,
                item = item_id,
                exit_code,
                stderr = err_msg,
                "take command failed, skipping item"
            );
            let namespace = {
                let mut workers = self.worker_states.lock();
                let ns = workers
                    .get(worker_name)
                    .map(|s| s.namespace.clone())
                    .unwrap_or_default();
                // Remove from inflight set so the item can be retried on next poll
                if let Some(state) = workers.get_mut(worker_name) {
                    state.inflight_items.remove(item_id);
                }
                ns
            };
            let scoped = scoped_name(&namespace, worker_name);
            self.worker_logger.append(
                &scoped,
                &format!(
                    "error: take command failed for item {}: {}",
                    item_id, err_msg
                ),
            );
            return Ok(vec![]);
        }

        let mut result_events = Vec::new();

        // Refresh runbook in case it changed while the take command was running
        if let Some(loaded_event) = self.refresh_worker_runbook(worker_name)? {
            result_events.push(loaded_event);
        }

        result_events.extend(self.dispatch_queue_item(worker_name, item).await?);

        Ok(result_events)
    }

    /// Create and dispatch a job for a single queue item.
    ///
    /// Shared by persisted-queue dispatch (inline in [`handle_worker_poll_complete`])
    /// and external-queue dispatch (deferred in [`handle_worker_take_complete`]).
    async fn dispatch_queue_item(
        &self,
        worker_name: &str,
        item: &serde_json::Value,
    ) -> Result<Vec<Event>, RuntimeError> {
        let mut result_events = Vec::new();

        let item_id = item
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let (job_kind, runbook_hash, cwd, worker_namespace) = {
            let workers = self.worker_states.lock();
            let state = match workers.get(worker_name) {
                Some(s) if s.status != WorkerStatus::Stopped => s,
                _ => return Ok(result_events),
            };
            (
                state.job_kind.clone(),
                state.runbook_hash.clone(),
                state.project_root.clone(),
                state.namespace.clone(),
            )
        };

        // Create job for this item
        let job_id = JobId::new(UuidIdGen.next());

        // Look up job definition to build input
        // Runbook refreshed at top of caller, no need to emit RunbookLoaded
        let runbook = self.cached_runbook(&runbook_hash)?;
        let job_def = runbook
            .get_job(&job_kind)
            .ok_or_else(|| RuntimeError::JobDefNotFound(job_kind.clone()))?;

        // Build input from item fields
        // Only create properly namespaced mappings:
        // - item.* (canonical namespace for queue item fields)
        // - ${first_var}.* (for backward compatibility with jobs declaring vars = ["epic"])
        let mut input = HashMap::new();
        input.insert("invoke.dir".to_string(), cwd.display().to_string());
        if let Some(obj) = item.as_object() {
            for (key, value) in obj {
                let v = if let Some(s) = value.as_str() {
                    s.to_string()
                } else {
                    value.to_string()
                };
                input.insert(format!("item.{}", key), v.clone());
                // Map fields into the namespace of the job's first declared var
                // e.g. if vars = ["bug"], map "bug.title", "bug.id", etc.
                if let Some(first_input) = job_def.vars.first() {
                    input.insert(format!("{}.{}", first_input, key), v);
                }
            }
        }

        // Build job name
        let name = format!("{}-{}", job_kind, item_id);

        result_events.extend(
            self.create_and_start_job(CreateJobParams {
                job_id: job_id.clone(),
                job_name: name,
                job_kind: job_kind.clone(),
                vars: input,
                runbook_hash: runbook_hash.clone(),
                runbook_json: None,
                runbook,
                namespace: worker_namespace.clone(),
                cron_name: None,
            })
            .await?,
        );

        // Track job in worker state and item-job mapping
        {
            let mut workers = self.worker_states.lock();
            if let Some(state) = workers.get_mut(worker_name) {
                state.active_jobs.insert(job_id.clone());
                state.item_job_map.insert(job_id.clone(), item_id.clone());
            }
        }

        // Emit WorkerItemDispatched
        let dispatch_event = Event::WorkerItemDispatched {
            worker_name: worker_name.to_string(),
            item_id: item_id.clone(),
            job_id: job_id.clone(),
            namespace: worker_namespace.clone(),
        };
        result_events.extend(
            self.executor
                .execute_all(vec![Effect::Emit {
                    event: dispatch_event,
                }])
                .await?,
        );

        let scoped = scoped_name(&worker_namespace, worker_name);
        self.worker_logger.append(
            &scoped,
            &format!(
                "dispatched item {} \u{2192} job {}",
                item_id,
                job_id.as_str()
            ),
        );

        Ok(result_events)
    }
}
