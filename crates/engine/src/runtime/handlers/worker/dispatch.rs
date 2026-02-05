// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue item dispatch: take items from queue and create pipelines

use super::WorkerStatus;
use crate::error::RuntimeError;
use crate::runtime::handlers::CreatePipelineParams;
use crate::runtime::Runtime;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{scoped_name, Clock, Effect, Event, IdGen, PipelineId, UuidIdGen};
use oj_runbook::QueueType;
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

            let active = state.active_pipelines.len() as u32;
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

        for item in items.iter().take(available_slots) {
            let item_id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            match queue_type {
                QueueType::External => {
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

                    // Fire take command as background task. Pipeline creation is
                    // deferred to handle_worker_take_complete when the command
                    // succeeds.
                    self.executor
                        .execute(Effect::TakeQueueItem {
                            worker_name: worker_name.to_string(),
                            take_command,
                            cwd: cwd.clone(),
                            item: item.clone(),
                        })
                        .await?;
                }
                QueueType::Persisted => {
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

                    // Dispatch pipeline immediately for persisted queues
                    result_events.extend(self.dispatch_queue_item(worker_name, item).await?);
                }
            }
        }

        Ok(result_events)
    }

    /// Handle a successful take command: create and dispatch a pipeline for the item.
    pub(crate) async fn handle_worker_take_complete(
        &self,
        worker_name: &str,
        item: &serde_json::Value,
    ) -> Result<Vec<Event>, RuntimeError> {
        let mut result_events = Vec::new();

        // Refresh runbook in case it changed while the take command was running
        if let Some(loaded_event) = self.refresh_worker_runbook(worker_name)? {
            result_events.push(loaded_event);
        }

        result_events.extend(self.dispatch_queue_item(worker_name, item).await?);

        Ok(result_events)
    }

    /// Handle a failed take command: log the error.
    pub(crate) async fn handle_worker_take_failed(
        &self,
        worker_name: &str,
        item_id: &str,
        error: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let namespace = {
            let workers = self.worker_states.lock();
            workers
                .get(worker_name)
                .map(|s| s.namespace.clone())
                .unwrap_or_default()
        };

        let scoped = scoped_name(&namespace, worker_name);
        self.worker_logger.append(
            &scoped,
            &format!("error: take command failed for item {}: {}", item_id, error),
        );

        Ok(vec![])
    }

    /// Create and dispatch a pipeline for a single queue item.
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

        let (pipeline_kind, runbook_hash, cwd, worker_namespace) = {
            let workers = self.worker_states.lock();
            let state = match workers.get(worker_name) {
                Some(s) if s.status != WorkerStatus::Stopped => s,
                _ => return Ok(result_events),
            };
            (
                state.pipeline_kind.clone(),
                state.runbook_hash.clone(),
                state.project_root.clone(),
                state.namespace.clone(),
            )
        };

        // Create pipeline for this item
        let pipeline_id = PipelineId::new(UuidIdGen.next());

        // Look up pipeline definition to build input
        let runbook = self.cached_runbook(&runbook_hash)?;
        let pipeline_def = runbook
            .get_pipeline(&pipeline_kind)
            .ok_or_else(|| RuntimeError::PipelineDefNotFound(pipeline_kind.clone()))?;

        // Build input from item fields
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
                input.insert(key.clone(), v.clone());
                // Map fields into the namespace of the pipeline's first declared var
                // e.g. if vars = ["bug"], map "bug.title", "bug.id", etc.
                if let Some(first_input) = pipeline_def.vars.first() {
                    input.insert(format!("{}.{}", first_input, key), v);
                }
            }
        }

        // Build pipeline name
        let name = format!("{}-{}", pipeline_kind, item_id);

        // Runbook refreshed at top of caller, no need to emit RunbookLoaded
        result_events.extend(
            self.create_and_start_pipeline(CreatePipelineParams {
                pipeline_id: pipeline_id.clone(),
                pipeline_name: name,
                pipeline_kind: pipeline_kind.clone(),
                vars: input,
                runbook_hash: runbook_hash.clone(),
                runbook_json: None,
                runbook,
                namespace: worker_namespace.clone(),
                cron_name: None,
            })
            .await?,
        );

        // Track pipeline in worker state and item-pipeline mapping
        {
            let mut workers = self.worker_states.lock();
            if let Some(state) = workers.get_mut(worker_name) {
                state.active_pipelines.insert(pipeline_id.clone());
                if state.queue_type == QueueType::Persisted {
                    state
                        .item_pipeline_map
                        .insert(pipeline_id.clone(), item_id.clone());
                }
            }
        }

        // Emit WorkerItemDispatched
        let dispatch_event = Event::WorkerItemDispatched {
            worker_name: worker_name.to_string(),
            item_id: item_id.clone(),
            pipeline_id: pipeline_id.clone(),
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
                "dispatched item {} → pipeline {}",
                item_id,
                pipeline_id.as_str()
            ),
        );

        Ok(result_events)
    }
}
