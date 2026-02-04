// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Materialized state from WAL replay

use oj_core::{
    pipeline::AgentSignal, AgentRun, AgentRunStatus, Decision, DecisionId, Event, Pipeline,
    PipelineConfig, StepOutcome, StepStatus, WorkspaceStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn epoch_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Session record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub pipeline_id: String,
}

/// Workspace mode for lifecycle management
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceMode {
    /// Workspace is deleted on successful completion, kept on failure
    #[default]
    Ephemeral,
    /// Workspace is never automatically deleted
    Persistent,
}

/// Workspace record with lifecycle management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub path: PathBuf,
    /// Branch for the worktree (None means use HEAD)
    pub branch: Option<String>,
    /// Owner of the workspace (pipeline_id or worker_name)
    pub owner: Option<String>,
    /// Current lifecycle status
    pub status: WorkspaceStatus,
    /// Workspace mode (ephemeral = delete on success, persistent = never delete)
    #[serde(default)]
    pub mode: WorkspaceMode,
    /// Epoch milliseconds when workspace was created (0 for pre-existing workspaces)
    #[serde(default)]
    pub created_at_ms: u64,
}

/// A stored runbook snapshot for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRunbook {
    pub version: u32,
    pub data: serde_json::Value,
}

/// Record of a running worker for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRecord {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub project_root: PathBuf,
    pub runbook_hash: String,
    /// "running" or "stopped"
    pub status: String,
    #[serde(default)]
    pub active_pipeline_ids: Vec<String>,
    #[serde(default)]
    pub queue_name: String,
    #[serde(default)]
    pub concurrency: u32,
}

/// Status of a queue item through its lifecycle
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueItemStatus {
    Pending,
    Active,
    Completed,
    Failed,
    Dead,
}

/// A single item in a persisted queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub queue_name: String,
    pub data: HashMap<String, String>,
    pub status: QueueItemStatus,
    pub worker_name: Option<String>,
    pub pushed_at_epoch_ms: u64,
    /// Number of times this item has failed (for retry tracking)
    #[serde(default)]
    pub failure_count: u32,
}

/// Record of a running cron for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRecord {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub project_root: PathBuf,
    pub runbook_hash: String,
    /// "running" or "stopped"
    pub status: String,
    pub interval: String,
    /// Deprecated: use run_target. Kept for backward compat.
    pub pipeline_name: String,
    /// What this cron runs: "pipeline:name" or "agent:name"
    #[serde(default)]
    pub run_target: String,
    /// Epoch ms when the cron was started (timer began)
    #[serde(default)]
    pub started_at_ms: u64,
    /// Epoch ms when the cron last fired (spawned a pipeline)
    #[serde(default)]
    pub last_fired_at_ms: Option<u64>,
}

/// Materialized state built from WAL operations
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MaterializedState {
    pub pipelines: HashMap<String, Pipeline>,
    pub sessions: HashMap<String, Session>,
    pub workspaces: HashMap<String, Workspace>,
    #[serde(default)]
    pub runbooks: HashMap<String, StoredRunbook>,
    #[serde(default)]
    pub workers: HashMap<String, WorkerRecord>,
    #[serde(default)]
    pub queue_items: HashMap<String, Vec<QueueItem>>,
    #[serde(default)]
    pub crons: HashMap<String, CronRecord>,
    #[serde(default)]
    pub decisions: HashMap<String, Decision>,
    #[serde(default)]
    pub agent_runs: HashMap<String, AgentRun>,
}

/// Build a composite key for namespace-scoped lookups.
///
/// When namespace is empty (backward compat with old events), returns the bare name.
fn scoped_key(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", namespace, name)
    }
}

impl MaterializedState {
    /// Get a pipeline by ID or unique prefix (like git commit hashes)
    pub fn get_pipeline(&self, id: &str) -> Option<&Pipeline> {
        // Try exact match first
        if let Some(pipeline) = self.pipelines.get(id) {
            return Some(pipeline);
        }

        // Try prefix match
        let matches: Vec<_> = self
            .pipelines
            .iter()
            .filter(|(k, _)| k.starts_with(id))
            .collect();

        // Only return if exactly one match (unambiguous)
        if matches.len() == 1 {
            Some(matches[0].1)
        } else {
            None
        }
    }

    /// Find a pipeline by agent_id in its current step
    fn find_pipeline_by_agent_id(&mut self, agent_id: &str) -> Option<&mut Pipeline> {
        self.pipelines
            .values_mut()
            .find(|p| p.step_history.last().and_then(|r| r.agent_id.as_deref()) == Some(agent_id))
    }

    /// Get a decision by ID or unique prefix
    pub fn get_decision(&self, id: &str) -> Option<&Decision> {
        if let Some(decision) = self.decisions.get(id) {
            return Some(decision);
        }
        let matches: Vec<_> = self
            .decisions
            .iter()
            .filter(|(k, _)| k.starts_with(id))
            .collect();
        if matches.len() == 1 {
            Some(matches[0].1)
        } else {
            None
        }
    }

    /// Look up the known project root for a namespace from workers and crons.
    ///
    /// Returns None if no entity with this namespace has been registered yet.
    pub fn project_root_for_namespace(&self, namespace: &str) -> Option<std::path::PathBuf> {
        for w in self.workers.values() {
            if w.namespace == namespace {
                return Some(w.project_root.clone());
            }
        }
        for c in self.crons.values() {
            if c.namespace == namespace {
                return Some(c.project_root.clone());
            }
        }
        None
    }

    /// Apply an event to derive state changes.
    ///
    /// This is the event-sourcing approach where state is derived from events.
    /// Events are facts about what happened; state is derived from those facts.
    pub fn apply_event(&mut self, event: &Event) {
        match event {
            Event::AgentWorking { agent_id } => {
                let pipeline_id = agent_id.as_str().to_string();
                if let Some(pipeline) = self.pipelines.get_mut(&pipeline_id) {
                    pipeline.step_status = StepStatus::Running;
                }
            }
            Event::AgentWaiting { .. } => {
                // Agent is idle, but still running - no state change needed
            }
            Event::AgentExited {
                agent_id,
                exit_code,
            } => {
                let pipeline_id = agent_id.as_str().to_string();
                if let Some(pipeline) = self.pipelines.get_mut(&pipeline_id) {
                    if *exit_code == Some(0) {
                        pipeline.step_status = StepStatus::Completed;
                    } else {
                        pipeline.step_status = StepStatus::Failed;
                        pipeline.error = Some(format!("exit code: {:?}", exit_code));
                    }
                }
            }
            Event::AgentFailed { agent_id, error } => {
                let pipeline_id = agent_id.as_str().to_string();
                if let Some(pipeline) = self.pipelines.get_mut(&pipeline_id) {
                    pipeline.step_status = StepStatus::Failed;
                    pipeline.error = Some(error.to_string());
                }
            }
            Event::AgentGone { agent_id } => {
                let pipeline_id = agent_id.as_str().to_string();
                if let Some(pipeline) = self.pipelines.get_mut(&pipeline_id) {
                    pipeline.step_status = StepStatus::Failed;
                    pipeline.error = Some("session terminated unexpectedly".to_string());
                }
            }

            Event::ShellExited {
                pipeline_id,
                step: _,
                exit_code,
            } => {
                if let Some(pipeline) = self.pipelines.get_mut(pipeline_id.as_str()) {
                    let now = epoch_ms_now();
                    if *exit_code == 0 {
                        pipeline.step_status = StepStatus::Completed;
                        pipeline.finalize_current_step(StepOutcome::Completed, now);
                    } else {
                        let error_msg = format!("shell exit code: {}", exit_code);
                        pipeline.step_status = StepStatus::Failed;
                        pipeline.error = Some(error_msg.clone());
                        pipeline.finalize_current_step(StepOutcome::Failed(error_msg), now);
                    }
                }
            }

            // === Typed state mutations ===
            Event::PipelineCreated {
                id,
                kind,
                name,
                runbook_hash,
                cwd,
                vars,
                initial_step,
                created_at_epoch_ms,
                namespace,
                cron_name,
            } => {
                let config = PipelineConfig {
                    id: id.to_string(),
                    name: name.clone(),
                    kind: kind.clone(),
                    vars: vars.clone(),
                    runbook_hash: runbook_hash.clone(),
                    cwd: cwd.clone(),
                    initial_step: initial_step.clone(),
                    namespace: namespace.clone(),
                    cron_name: cron_name.clone(),
                };
                let pipeline = Pipeline::new_with_epoch_ms(config, *created_at_epoch_ms);
                self.pipelines.insert(id.to_string(), pipeline);
            }

            Event::RunbookLoaded {
                hash,
                version,
                runbook,
            } => {
                // Only insert if not already present (dedup by content hash)
                if !self.runbooks.contains_key(hash) {
                    self.runbooks.insert(
                        hash.clone(),
                        StoredRunbook {
                            version: *version,
                            data: runbook.clone(),
                        },
                    );
                }
            }

            Event::PipelineAdvanced { id, step } => {
                if let Some(pipeline) = self.pipelines.get_mut(id.as_str()) {
                    // Idempotency: skip if already on this step, UNLESS recovering
                    // from failure (on_fail → same step cycle).
                    let is_failure_transition = pipeline.step_status == StepStatus::Failed;
                    if pipeline.step == *step && !is_failure_transition {
                        return;
                    }
                    // Clear stale error and session when resuming from terminal state
                    let was_terminal = pipeline.is_terminal();
                    let target_is_nonterminal =
                        step != "done" && step != "failed" && step != "cancelled";
                    if was_terminal && target_is_nonterminal {
                        pipeline.error = None;
                        pipeline.session_id = None;
                    }

                    let now = epoch_ms_now();
                    // Finalize the previous step
                    let outcome = match step.as_str() {
                        "failed" | "cancelled" => {
                            StepOutcome::Failed(pipeline.error.clone().unwrap_or_default())
                        }
                        _ => StepOutcome::Completed,
                    };
                    pipeline.finalize_current_step(outcome, now);

                    pipeline.step = step.clone();
                    pipeline.step_status = match step.as_str() {
                        "failed" | "cancelled" => StepStatus::Failed,
                        "done" => StepStatus::Completed,
                        _ => StepStatus::Pending,
                    };

                    // Only reset action attempts on success transitions.
                    // On failure (on_fail) transitions, preserve attempts so that
                    // cycle limits work — the agent action's `attempts` field should
                    // bound retries across the entire on_fail chain, not per-step.
                    if !is_failure_transition {
                        pipeline.reset_action_attempts();
                    }
                    pipeline.clear_agent_signal();

                    // Push new step record and track visits (unless terminal)
                    if step != "done" && step != "failed" && step != "cancelled" {
                        pipeline.record_step_visit(step);
                        pipeline.push_step(step, now);
                    }
                }

                // Remove from worker active_pipeline_ids on terminal states
                if step == "done" || step == "failed" || step == "cancelled" {
                    let pipeline_id_str = id.to_string();
                    for record in self.workers.values_mut() {
                        record
                            .active_pipeline_ids
                            .retain(|pid| pid != &pipeline_id_str);
                    }
                }
            }

            Event::StepStarted {
                pipeline_id,
                agent_id,
                agent_name,
                ..
            } => {
                if let Some(pipeline) = self.pipelines.get_mut(pipeline_id.as_str()) {
                    pipeline.step_status = StepStatus::Running;
                    if let Some(aid) = agent_id {
                        pipeline.set_current_step_agent_id(aid.as_str());
                    }
                    if let Some(aname) = agent_name {
                        pipeline.set_current_step_agent_name(aname.as_str());
                    }
                    pipeline.update_current_step_outcome(StepOutcome::Running);
                }
            }

            Event::StepWaiting {
                pipeline_id,
                reason,
                decision_id,
                ..
            } => {
                if let Some(pipeline) = self.pipelines.get_mut(pipeline_id.as_str()) {
                    pipeline.step_status = StepStatus::Waiting(decision_id.clone());
                    if reason.is_some() {
                        pipeline.error.clone_from(reason);
                    }
                    let reason_str = reason.clone().unwrap_or_default();
                    pipeline.update_current_step_outcome(StepOutcome::Waiting(reason_str));
                }
            }

            Event::StepCompleted { pipeline_id, .. } => {
                if let Some(pipeline) = self.pipelines.get_mut(pipeline_id.as_str()) {
                    pipeline.step_status = StepStatus::Completed;
                    pipeline.finalize_current_step(StepOutcome::Completed, epoch_ms_now());
                }
            }

            Event::StepFailed {
                pipeline_id, error, ..
            } => {
                if let Some(pipeline) = self.pipelines.get_mut(pipeline_id.as_str()) {
                    pipeline.step_status = StepStatus::Failed;
                    pipeline.error = Some(error.clone());
                    pipeline
                        .finalize_current_step(StepOutcome::Failed(error.clone()), epoch_ms_now());
                }
            }

            Event::PipelineCancelling { id } => {
                if let Some(pipeline) = self.pipelines.get_mut(id.as_str()) {
                    pipeline.cancelling = true;
                }
            }

            Event::PipelineDeleted { id } => {
                self.pipelines.remove(id.as_str());
            }

            Event::SessionCreated {
                id,
                pipeline_id,
                agent_run_id,
            } => {
                self.sessions.insert(
                    id.to_string(),
                    Session {
                        id: id.to_string(),
                        pipeline_id: pipeline_id.to_string(),
                    },
                );
                // Update the pipeline's session_id
                if let Some(pipeline) = self.pipelines.get_mut(pipeline_id.as_str()) {
                    pipeline.session_id = Some(id.to_string());
                }
                // Update the agent_run's session_id (standalone agents)
                if let Some(ref ar_id) = agent_run_id {
                    if let Some(agent_run) = self.agent_runs.get_mut(ar_id.as_str()) {
                        agent_run.session_id = Some(id.to_string());
                    }
                }
            }

            Event::SessionDeleted { id } => {
                self.sessions.remove(id.as_str());
            }

            Event::WorkspaceCreated {
                id,
                path,
                branch,
                owner,
                mode,
            } => {
                let ws_mode = mode
                    .as_deref()
                    .map(|s| match s {
                        "persistent" => WorkspaceMode::Persistent,
                        _ => WorkspaceMode::Ephemeral,
                    })
                    .unwrap_or_default();

                // Also update the pipeline's workspace info if owner matches
                if let Some(ref owner_id) = owner {
                    if let Some(pipeline) = self.pipelines.get_mut(owner_id.as_str()) {
                        pipeline.workspace_path = Some(path.clone());
                        pipeline.workspace_id = Some(id.clone());
                    }
                }

                self.workspaces.insert(
                    id.to_string(),
                    Workspace {
                        id: id.to_string(),
                        path: path.clone(),
                        branch: branch.clone(),
                        owner: owner.clone(),
                        status: WorkspaceStatus::Creating,
                        mode: ws_mode,
                        created_at_ms: epoch_ms_now(),
                    },
                );
            }

            Event::WorkspaceReady { id } => {
                if let Some(workspace) = self.workspaces.get_mut(id.as_str()) {
                    workspace.status = WorkspaceStatus::Ready;
                }
            }

            Event::WorkspaceFailed { id, reason } => {
                if let Some(workspace) = self.workspaces.get_mut(id.as_str()) {
                    workspace.status = WorkspaceStatus::Failed {
                        reason: reason.clone(),
                    };
                }
            }

            Event::WorkspaceDeleted { id } => {
                self.workspaces.remove(id.as_str());
            }

            Event::PipelineUpdated { id, vars } => {
                if let Some(pipeline) = self.pipelines.get_mut(id.as_str()) {
                    for (key, value) in vars {
                        pipeline.vars.insert(key.clone(), value.clone());
                    }
                }
            }

            Event::AgentSignal {
                agent_id,
                kind,
                message,
            } => {
                // Check standalone agent runs first
                let found_agent_run = self
                    .agent_runs
                    .values_mut()
                    .find(|r| r.agent_id.as_deref() == Some(agent_id.as_str()));
                if let Some(run) = found_agent_run {
                    run.agent_signal = Some(AgentSignal {
                        kind: kind.clone(),
                        message: message.clone(),
                    });
                } else if let Some(pipeline) = self.find_pipeline_by_agent_id(agent_id.as_str()) {
                    // Find pipeline by agent_id in current step
                    pipeline.agent_signal = Some(AgentSignal {
                        kind: kind.clone(),
                        message: message.clone(),
                    });
                }
            }

            // -- worker events --
            Event::WorkerStarted {
                worker_name,
                project_root,
                runbook_hash,
                queue_name,
                concurrency,
                namespace,
            } => {
                let key = scoped_key(namespace, worker_name);
                // Preserve active_pipeline_ids from before restart
                let existing_pipeline_ids = self
                    .workers
                    .get(&key)
                    .map(|w| w.active_pipeline_ids.clone())
                    .unwrap_or_default();

                self.workers.insert(
                    key,
                    WorkerRecord {
                        name: worker_name.clone(),
                        namespace: namespace.clone(),
                        project_root: project_root.clone(),
                        runbook_hash: runbook_hash.clone(),
                        status: "running".to_string(),
                        active_pipeline_ids: existing_pipeline_ids,
                        queue_name: queue_name.clone(),
                        concurrency: *concurrency,
                    },
                );
            }

            Event::WorkerItemDispatched {
                worker_name,
                pipeline_id,
                namespace,
                ..
            } => {
                let key = scoped_key(namespace, worker_name);
                if let Some(record) = self.workers.get_mut(&key) {
                    let pid = pipeline_id.to_string();
                    if !record.active_pipeline_ids.contains(&pid) {
                        record.active_pipeline_ids.push(pid);
                    }
                }
            }

            Event::WorkerStopped {
                worker_name,
                namespace,
            } => {
                let key = scoped_key(namespace, worker_name);
                if let Some(record) = self.workers.get_mut(&key) {
                    record.status = "stopped".to_string();
                }
            }

            Event::WorkerDeleted {
                worker_name,
                namespace,
            } => {
                let key = scoped_key(namespace, worker_name);
                self.workers.remove(&key);
            }

            // -- queue events --
            Event::QueuePushed {
                queue_name,
                item_id,
                data,
                pushed_at_epoch_ms,
                namespace,
            } => {
                let key = scoped_key(namespace, queue_name);
                let items = self.queue_items.entry(key).or_default();
                // Idempotency: skip if item already exists
                if !items.iter().any(|i| i.id == *item_id) {
                    items.push(QueueItem {
                        id: item_id.clone(),
                        queue_name: queue_name.clone(),
                        data: data.clone(),
                        status: QueueItemStatus::Pending,
                        worker_name: None,
                        pushed_at_epoch_ms: *pushed_at_epoch_ms,
                        failure_count: 0,
                    });
                }
            }

            Event::QueueTaken {
                queue_name,
                item_id,
                worker_name,
                namespace,
            } => {
                let key = scoped_key(namespace, queue_name);
                if let Some(items) = self.queue_items.get_mut(&key) {
                    if let Some(item) = items.iter_mut().find(|i| i.id == *item_id) {
                        item.status = QueueItemStatus::Active;
                        item.worker_name = Some(worker_name.clone());
                    }
                }
            }

            Event::QueueCompleted {
                queue_name,
                item_id,
                namespace,
            } => {
                let key = scoped_key(namespace, queue_name);
                if let Some(items) = self.queue_items.get_mut(&key) {
                    if let Some(item) = items.iter_mut().find(|i| i.id == *item_id) {
                        item.status = QueueItemStatus::Completed;
                    }
                }
            }

            Event::QueueFailed {
                queue_name,
                item_id,
                namespace,
                ..
            } => {
                let key = scoped_key(namespace, queue_name);
                if let Some(items) = self.queue_items.get_mut(&key) {
                    if let Some(item) = items.iter_mut().find(|i| i.id == *item_id) {
                        item.status = QueueItemStatus::Failed;
                        item.failure_count += 1;
                    }
                }
            }

            Event::QueueDropped {
                queue_name,
                item_id,
                namespace,
            } => {
                let key = scoped_key(namespace, queue_name);
                if let Some(items) = self.queue_items.get_mut(&key) {
                    items.retain(|i| i.id != *item_id);
                }
            }

            Event::QueueItemRetry {
                queue_name,
                item_id,
                namespace,
            } => {
                let key = scoped_key(namespace, queue_name);
                if let Some(items) = self.queue_items.get_mut(&key) {
                    if let Some(item) = items.iter_mut().find(|i| i.id == *item_id) {
                        item.status = QueueItemStatus::Pending;
                        item.failure_count = 0;
                        item.worker_name = None;
                    }
                }
            }

            Event::QueueItemDead {
                queue_name,
                item_id,
                namespace,
            } => {
                let key = scoped_key(namespace, queue_name);
                if let Some(items) = self.queue_items.get_mut(&key) {
                    if let Some(item) = items.iter_mut().find(|i| i.id == *item_id) {
                        item.status = QueueItemStatus::Dead;
                    }
                }
            }

            // -- cron events --
            Event::CronStarted {
                cron_name,
                project_root,
                runbook_hash,
                interval,
                pipeline_name,
                run_target,
                namespace,
            } => {
                let key = scoped_key(namespace, cron_name);
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                // Preserve last_fired_at_ms across restarts (re-emitted CronStarted)
                let last_fired_at_ms = self.crons.get(&key).and_then(|r| r.last_fired_at_ms);
                // Use run_target if present, fall back to pipeline_name for backward compat
                let effective_target = if run_target.is_empty() {
                    format!("pipeline:{}", pipeline_name)
                } else {
                    run_target.clone()
                };
                self.crons.insert(
                    key,
                    CronRecord {
                        name: cron_name.clone(),
                        namespace: namespace.clone(),
                        project_root: project_root.clone(),
                        runbook_hash: runbook_hash.clone(),
                        status: "running".to_string(),
                        interval: interval.clone(),
                        pipeline_name: pipeline_name.clone(),
                        run_target: effective_target,
                        started_at_ms: now_ms,
                        last_fired_at_ms,
                    },
                );
            }

            Event::CronStopped {
                cron_name,
                namespace,
            } => {
                let key = scoped_key(namespace, cron_name);
                if let Some(record) = self.crons.get_mut(&key) {
                    record.status = "stopped".to_string();
                }
            }

            Event::CronFired {
                cron_name,
                namespace,
                ..
            } => {
                let key = scoped_key(namespace, cron_name);
                if let Some(record) = self.crons.get_mut(&key) {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    record.last_fired_at_ms = Some(now_ms);
                }
            }

            Event::CronDeleted {
                cron_name,
                namespace,
            } => {
                let key = scoped_key(namespace, cron_name);
                self.crons.remove(&key);
            }

            // -- decision events --
            Event::DecisionCreated {
                id,
                pipeline_id,
                agent_id,
                source,
                context,
                options,
                created_at_ms,
                namespace,
            } => {
                // Idempotency: skip if already exists
                if !self.decisions.contains_key(id) {
                    self.decisions.insert(
                        id.clone(),
                        Decision {
                            id: DecisionId::new(id.clone()),
                            pipeline_id: pipeline_id.to_string(),
                            agent_id: agent_id.clone(),
                            source: source.clone(),
                            context: context.clone(),
                            options: options.clone(),
                            chosen: None,
                            message: None,
                            created_at_ms: *created_at_ms,
                            resolved_at_ms: None,
                            namespace: namespace.clone(),
                        },
                    );
                }

                // Update pipeline step status to Waiting with decision_id
                if let Some(pipeline) = self.pipelines.get_mut(pipeline_id.as_str()) {
                    pipeline.step_status = StepStatus::Waiting(Some(id.clone()));
                }
            }

            Event::DecisionResolved {
                id,
                chosen,
                message,
                resolved_at_ms,
                ..
            } => {
                if let Some(decision) = self.decisions.get_mut(id) {
                    decision.chosen = *chosen;
                    decision.message.clone_from(message);
                    decision.resolved_at_ms = Some(*resolved_at_ms);
                }
            }

            // -- agent_run events --
            Event::AgentRunCreated {
                id,
                agent_name,
                command_name,
                namespace,
                cwd,
                runbook_hash,
                vars,
                created_at_epoch_ms,
            } => {
                self.agent_runs.insert(
                    id.as_str().to_string(),
                    AgentRun {
                        id: id.as_str().to_string(),
                        agent_name: agent_name.clone(),
                        command_name: command_name.clone(),
                        namespace: namespace.clone(),
                        cwd: cwd.clone(),
                        runbook_hash: runbook_hash.clone(),
                        status: AgentRunStatus::Starting,
                        agent_id: None,
                        session_id: None,
                        error: None,
                        created_at_ms: *created_at_epoch_ms,
                        updated_at_ms: *created_at_epoch_ms,
                        action_attempts: HashMap::new(),
                        agent_signal: None,
                        vars: vars.clone(),
                    },
                );
            }

            Event::AgentRunStarted { id, agent_id } => {
                if let Some(run) = self.agent_runs.get_mut(id.as_str()) {
                    run.status = AgentRunStatus::Running;
                    run.agent_id = Some(agent_id.as_str().to_string());
                    run.updated_at_ms = epoch_ms_now();
                }
            }

            Event::AgentRunStatusChanged { id, status, reason } => {
                if let Some(run) = self.agent_runs.get_mut(id.as_str()) {
                    run.status = status.clone();
                    if let Some(reason) = reason {
                        run.error = Some(reason.clone());
                    }
                    run.updated_at_ms = epoch_ms_now();
                }
            }

            Event::AgentRunDeleted { id } => {
                self.agent_runs.remove(id.as_str());
            }

            // Events that don't affect persisted state
            // (These are action/signal events handled by the runtime)
            Event::Custom
            | Event::CommandRun { .. }
            | Event::TimerStart { .. }
            | Event::SessionInput { .. }
            | Event::AgentInput { .. }
            | Event::PipelineResume { .. }
            | Event::PipelineCancel { .. }
            | Event::WorkspaceDrop { .. }
            | Event::WorkerWake { .. }
            | Event::WorkerPollComplete { .. }
            | Event::AgentIdle { .. }
            | Event::AgentPrompt { .. }
            | Event::CronOnce { .. }
            | Event::Shutdown => {}
        }
    }
}

#[cfg(test)]
#[path = "state_tests/mod.rs"]
mod tests;
