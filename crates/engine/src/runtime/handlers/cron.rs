// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron event handling

use super::super::Runtime;
use super::CreatePipelineParams;
use crate::error::RuntimeError;
use oj_adapters::{AgentAdapter, NotifyAdapter, SessionAdapter};
use oj_core::{Clock, Effect, Event, IdGen, PipelineId, TimerId, UuidIdGen};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// In-memory state for a running cron
pub(crate) struct CronState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub interval: String,
    pub pipeline_name: String,
    pub status: CronStatus,
    pub namespace: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CronStatus {
    Running,
    Stopped,
}

impl<S, A, N, C> Runtime<S, A, N, C>
where
    S: SessionAdapter,
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    pub(crate) async fn handle_cron_started(
        &self,
        cron_name: &str,
        project_root: &Path,
        runbook_hash: &str,
        interval: &str,
        pipeline_name: &str,
        namespace: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        let duration = crate::monitor::parse_duration(interval).map_err(|e| {
            RuntimeError::InvalidFormat(format!("invalid cron interval '{}': {}", interval, e))
        })?;

        // Store cron state
        let state = CronState {
            project_root: project_root.to_path_buf(),
            runbook_hash: runbook_hash.to_string(),
            interval: interval.to_string(),
            pipeline_name: pipeline_name.to_string(),
            status: CronStatus::Running,
            namespace: namespace.to_string(),
        };

        {
            let mut crons = self.cron_states.lock();
            crons.insert(cron_name.to_string(), state);
        }

        // Set the first interval timer
        let timer_id = TimerId::cron(cron_name, namespace);
        self.executor
            .execute(Effect::SetTimer {
                id: timer_id,
                duration,
            })
            .await?;

        Ok(vec![])
    }

    pub(crate) async fn handle_cron_stopped(
        &self,
        cron_name: &str,
        namespace: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        {
            let mut crons = self.cron_states.lock();
            if let Some(state) = crons.get_mut(cron_name) {
                state.status = CronStatus::Stopped;
            }
        }

        // Cancel the timer
        let timer_id = TimerId::cron(cron_name, namespace);
        self.executor
            .execute(Effect::CancelTimer { id: timer_id })
            .await?;

        Ok(vec![])
    }

    /// Handle a cron timer firing: spawn pipeline and reschedule timer.
    pub(crate) async fn handle_cron_timer_fired(
        &self,
        rest: &str,
    ) -> Result<Vec<Event>, RuntimeError> {
        // Parse cron name from timer ID rest (after "cron:" prefix)
        // Format: "cron_name" or "namespace/cron_name"
        let cron_name = rest.rsplit('/').next().unwrap_or(rest);

        let (_project_root, runbook_hash, pipeline_name, interval, namespace) = {
            let crons = self.cron_states.lock();
            match crons.get(cron_name) {
                Some(s) if s.status == CronStatus::Running => (
                    s.project_root.clone(),
                    s.runbook_hash.clone(),
                    s.pipeline_name.clone(),
                    s.interval.clone(),
                    s.namespace.clone(),
                ),
                _ => {
                    tracing::debug!(cron = cron_name, "cron timer fired but cron not running");
                    return Ok(vec![]);
                }
            }
        };

        // Refresh runbook from disk
        if let Some(loaded_event) = self.refresh_cron_runbook(cron_name)? {
            // Process the loaded event to update caches
            let _ = self
                .executor
                .execute_all(vec![Effect::Emit {
                    event: loaded_event,
                }])
                .await?;
        }

        // Re-read hash after potential refresh
        let runbook_hash = {
            let crons = self.cron_states.lock();
            crons
                .get(cron_name)
                .map(|s| s.runbook_hash.clone())
                .unwrap_or(runbook_hash)
        };

        let runbook = self.cached_runbook(&runbook_hash)?;

        // Generate pipeline ID
        let pipeline_id = PipelineId::new(UuidIdGen.next());
        let display_name = oj_runbook::pipeline_display_name(
            &pipeline_name,
            &pipeline_id.as_str()[..8.min(pipeline_id.as_str().len())],
        );

        let mut result_events = Vec::new();

        // Create and start pipeline
        result_events.extend(
            self.create_and_start_pipeline(CreatePipelineParams {
                pipeline_id: pipeline_id.clone(),
                pipeline_name: display_name,
                pipeline_kind: pipeline_name.clone(),
                vars: HashMap::new(),
                runbook_hash: runbook_hash.clone(),
                runbook_json: None,
                runbook,
                namespace: namespace.clone(),
            })
            .await?,
        );

        // Emit CronFired tracking event
        result_events.extend(
            self.executor
                .execute_all(vec![Effect::Emit {
                    event: Event::CronFired {
                        cron_name: cron_name.to_string(),
                        pipeline_id,
                        namespace: namespace.clone(),
                    },
                }])
                .await?,
        );

        // Reschedule timer for next interval
        let duration = crate::monitor::parse_duration(&interval).map_err(|e| {
            RuntimeError::InvalidFormat(format!("invalid cron interval '{}': {}", interval, e))
        })?;
        let timer_id = TimerId::cron(cron_name, &namespace);
        self.executor
            .execute(Effect::SetTimer {
                id: timer_id,
                duration,
            })
            .await?;

        Ok(result_events)
    }

    /// Re-read the runbook from disk for a running cron.
    ///
    /// If the content has changed since last cached, updates the in-process
    /// cache and cron state, and returns a `RunbookLoaded` event for WAL
    /// persistence. Returns `Ok(None)` when the runbook is unchanged.
    fn refresh_cron_runbook(
        &self,
        cron_name: &str,
    ) -> Result<Option<oj_core::Event>, RuntimeError> {
        let project_root = {
            let crons = self.cron_states.lock();
            match crons.get(cron_name) {
                Some(s) => s.project_root.clone(),
                None => return Ok(None),
            }
        };

        // Load runbook from disk
        let runbook_dir = project_root.join(".oj/runbooks");
        let runbook = oj_runbook::find_runbook_by_cron(&runbook_dir, cron_name)
            .map_err(|e| RuntimeError::RunbookLoadError(e.to_string()))?
            .ok_or_else(|| {
                RuntimeError::RunbookLoadError(format!(
                    "no runbook found containing cron '{}'",
                    cron_name
                ))
            })?;

        // Compute content hash
        let runbook_json = serde_json::to_value(&runbook)
            .map_err(|e| RuntimeError::RunbookLoadError(format!("failed to serialize: {}", e)))?;
        let runbook_hash = {
            use sha2::{Digest, Sha256};
            let canonical = serde_json::to_string(&runbook_json).map_err(|e| {
                RuntimeError::RunbookLoadError(format!("failed to serialize: {}", e))
            })?;
            let digest = Sha256::digest(canonical.as_bytes());
            format!("{:x}", digest)
        };

        // Check if hash changed
        let old_hash = {
            let crons = self.cron_states.lock();
            crons
                .get(cron_name)
                .map(|s| s.runbook_hash.clone())
                .unwrap_or_default()
        };

        if old_hash == runbook_hash {
            return Ok(None);
        }

        tracing::info!(
            cron = cron_name,
            old_hash = &old_hash[..12.min(old_hash.len())],
            new_hash = &runbook_hash[..12.min(runbook_hash.len())],
            "runbook changed on disk, refreshing"
        );

        // Update cron state
        {
            let mut crons = self.cron_states.lock();
            if let Some(state) = crons.get_mut(cron_name) {
                state.runbook_hash = runbook_hash.clone();
            }
        }

        // Update in-process cache
        {
            let mut cache = self.runbook_cache.lock();
            cache.insert(runbook_hash.clone(), runbook);
        }

        // Return RunbookLoaded event for WAL persistence
        Ok(Some(oj_core::Event::RunbookLoaded {
            hash: runbook_hash,
            version: 1,
            runbook: runbook_json,
        }))
    }
}
