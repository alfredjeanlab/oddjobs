// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker event handling

use super::super::Runtime;
use super::CreatePipelineParams;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, IdGen, PipelineId, UuidIdGen};
use oj_runbook::QueueType;
use oj_storage::QueueItemStatus;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// In-memory state for a running worker
pub(crate) struct WorkerState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub queue_name: String,
    pub pipeline_kind: String,
    pub concurrency: u32,
    pub active_pipelines: HashSet<PipelineId>,
    pub status: WorkerStatus,
    pub queue_type: QueueType,
    /// Maps pipeline_id -> item_id for persisted queue item completion tracking
    pub item_pipeline_map: HashMap<PipelineId, String>,
    /// Project namespace
    pub namespace: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerStatus {
    Running,
    Stopped,
}

impl<S, A, C> Runtime<S, A, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
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

        // Restore active pipelines from persisted state (survives daemon restart)
        let (persisted_active, persisted_item_map) = self.lock_state(|state| {
            let active: HashSet<PipelineId> = state
                .workers
                .get(worker_name)
                .map(|w| w.active_pipeline_ids.iter().map(PipelineId::new).collect())
                .unwrap_or_default();

            let item_map: HashMap<PipelineId, String> = if queue_type == QueueType::Persisted {
                active
                    .iter()
                    .filter_map(|pid| {
                        state
                            .pipelines
                            .get(pid.as_str())
                            .and_then(|p| p.vars.get("item.id"))
                            .map(|item_id| (pid.clone(), item_id.clone()))
                    })
                    .collect()
            } else {
                HashMap::new()
            };

            (active, item_map)
        });

        // Store worker state
        let state = WorkerState {
            project_root: project_root.to_path_buf(),
            runbook_hash: runbook_hash.to_string(),
            queue_name: worker_def.source.queue.clone(),
            pipeline_kind: worker_def.handler.pipeline.clone(),
            concurrency: worker_def.concurrency,
            active_pipelines: persisted_active,
            status: WorkerStatus::Running,
            queue_type,
            item_pipeline_map: persisted_item_map,
            namespace: namespace.to_string(),
        };

        {
            let mut workers = self.worker_states.lock();
            workers.insert(worker_name.to_string(), state);
        }

        // Trigger initial poll
        match queue_type {
            QueueType::External => {
                let list_command = queue_def.list.clone().unwrap_or_default();
                Ok(self
                    .executor
                    .execute_all(vec![Effect::PollQueue {
                        worker_name: worker_name.to_string(),
                        list_command,
                        cwd: project_root.to_path_buf(),
                    }])
                    .await?)
            }
            QueueType::Persisted => {
                self.poll_persisted_queue(worker_name, &worker_def.source.queue)
            }
        }
    }

    pub(crate) async fn handle_worker_wake(
        &self,
        worker_name: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        tracing::info!(worker = worker_name, "worker wake");
        let mut result_events = Vec::new();

        // Refresh runbook from disk so edits after `oj worker start` are picked up
        if let Some(loaded_event) = self.refresh_worker_runbook(worker_name)? {
            result_events.push(loaded_event);
        }

        let (queue_type, queue_name, runbook_hash, project_root) = {
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
            }
            QueueType::Persisted => {
                result_events.extend(self.poll_persisted_queue(worker_name, &queue_name)?);
            }
        }

        Ok(result_events)
    }

    /// Read pending items from MaterializedState and synthesize a WorkerPollComplete event.
    fn poll_persisted_queue(
        &self,
        worker_name: &str,
        queue_name: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let (total, items): (usize, Vec<serde_json::Value>) = self.lock_state(|state| match state
            .queue_items
            .get(queue_name)
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

        let (
            queue_type,
            take_template,
            cwd,
            available_slots,
            pipeline_kind,
            runbook_hash,
            queue_name,
            worker_namespace,
        ) = {
            let mut workers = self.worker_states.lock();
            let state = match workers.get_mut(worker_name) {
                Some(s) if s.status != WorkerStatus::Stopped => s,
                _ => return Ok(result_events),
            };

            let active = state.active_pipelines.len() as u32;
            let available = state.concurrency.saturating_sub(active);
            if available == 0 || items.is_empty() {
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
                state.pipeline_kind.clone(),
                state.runbook_hash.clone(),
                state.queue_name.clone(),
                state.namespace.clone(),
            )
        };

        for item in items.iter().take(available_slots) {
            // Extract item_id for tracking
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

                    // Execute take command
                    let take_result = self
                        .executor
                        .execute(Effect::TakeQueueItem {
                            worker_name: worker_name.to_string(),
                            take_command,
                            cwd: cwd.clone(),
                        })
                        .await;

                    if let Err(e) = take_result {
                        tracing::warn!(
                            worker = worker_name,
                            error = %e,
                            "take command failed, skipping item"
                        );
                        continue;
                    }
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
                }
            }

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

            // Runbook refreshed at top of handle_worker_poll_complete, no need to emit RunbookLoaded
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
        }

        Ok(result_events)
    }

    pub(crate) async fn handle_worker_stopped(
        &self,
        worker_name: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let mut workers = self.worker_states.lock();
        if let Some(state) = workers.get_mut(worker_name) {
            state.status = WorkerStatus::Stopped;
        }
        Ok(vec![])
    }

    /// Check if a completed pipeline belongs to a worker and trigger re-poll if so.
    /// For persisted queues, also emits queue:completed or queue:failed events.
    pub(crate) async fn check_worker_pipeline_complete(
        &self,
        pipeline_id: &PipelineId,
        terminal_step: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Find which worker (if any) owns this pipeline
        let worker_info = {
            let mut workers = self.worker_states.lock();
            let mut found = None;
            for (name, state) in workers.iter_mut() {
                if state.active_pipelines.remove(pipeline_id) {
                    let item_id = state.item_pipeline_map.remove(pipeline_id);
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

            // For persisted queues, emit queue completion/failure event
            if queue_type == QueueType::Persisted {
                if let Some(item_id) = item_id {
                    let queue_event = if terminal_step == "done" {
                        Event::QueueCompleted {
                            queue_name: queue_name.clone(),
                            item_id,
                            namespace: worker_namespace.clone(),
                        }
                    } else {
                        Event::QueueFailed {
                            queue_name: queue_name.clone(),
                            item_id,
                            error: format!("pipeline reached '{}'", terminal_step),
                            namespace: worker_namespace.clone(),
                        }
                    };
                    result_events.extend(
                        self.executor
                            .execute_all(vec![Effect::Emit { event: queue_event }])
                            .await?,
                    );
                }
            }

            // Check if worker is still running and has capacity
            let should_poll = {
                let workers = self.worker_states.lock();
                workers
                    .get(&worker_name)
                    .map(|s| {
                        s.status == WorkerStatus::Running
                            && (s.active_pipelines.len() as u32) < s.concurrency
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
                        result_events.extend(self.poll_persisted_queue(&worker_name, &queue_name)?);
                    }
                }
            }
        }

        Ok(result_events)
    }
}
