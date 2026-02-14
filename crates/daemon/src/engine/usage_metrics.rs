// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent usage metrics collection.
//!
//! Periodically queries coop's usage API for token counts and writes
//! cumulative records to an append-only JSONL file at
//! `~/.local/state/oj/metrics/usage.jsonl`.
//!
//! The collector runs as a background tokio task and writes frequently
//! enough that cost data survives daemon crashes.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::engine::time_fmt::format_utc_now;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::adapters::agent::{AgentAdapter, UsageData};
use crate::storage::MaterializedState;
use oj_core::{MetricsHealth, OwnerId};

/// Default collection interval (30 seconds).
const DEFAULT_INTERVAL_SECS: u64 = 30;

/// Maximum metrics file size before rotation (10 MB).
const MAX_METRICS_SIZE: u64 = 10 * 1024 * 1024;

/// Number of rotated files to keep (usage.jsonl.1, .2, .3).
const MAX_ROTATED_FILES: u32 = 3;

/// A single usage record written to the JSONL metrics file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub timestamp: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub status: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_api_ms: Option<u64>,
}

/// Background metrics collector.
pub struct UsageMetricsCollector {
    state: Arc<Mutex<MaterializedState>>,
    agents: Arc<dyn AgentAdapter>,
    metrics_dir: PathBuf,
    /// Metadata enrichment: agent_id -> (agent_kind, job_id, job_kind, job_step, project, status)
    agent_meta: std::collections::HashMap<String, AgentMeta>,
    /// Cached usage per agent (from adapter API responses)
    cached_usage: std::collections::HashMap<String, UsageData>,
    health: Arc<Mutex<MetricsHealth>>,
}

struct AgentMeta {
    agent_kind: Option<String>,
    job_id: Option<String>,
    job_kind: Option<String>,
    job_step: Option<String>,
    project: Option<String>,
    status: String,
}

impl UsageMetricsCollector {
    /// Spawn the background metrics collector task.
    ///
    /// Returns a shared health handle for the listener to query.
    pub fn spawn_collector(
        state: Arc<Mutex<MaterializedState>>,
        agents: Arc<dyn AgentAdapter>,
        metrics_dir: PathBuf,
    ) -> Arc<Mutex<MetricsHealth>> {
        let health = Arc::new(Mutex::new(MetricsHealth::default()));

        let mut collector = UsageMetricsCollector {
            state,
            agents,
            metrics_dir,
            agent_meta: std::collections::HashMap::new(),
            cached_usage: std::collections::HashMap::new(),
            health: Arc::clone(&health),
        };

        let interval_secs = std::env::var("OJ_METRICS_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_INTERVAL_SECS);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

            loop {
                interval.tick().await;
                collector.collect_once().await;
            }
        });

        health
    }

    /// Run one collection cycle: snapshot state, query coop APIs, write records.
    async fn collect_once(&mut self) {
        // Snapshot agents and jobs from state (brief lock)
        let (agents, jobs) = {
            let state = self.state.lock();
            (state.agents.clone(), state.jobs.clone())
        };

        // Update metadata and query usage from coop
        self.agent_meta.clear();
        for record in agents.values() {
            let (job_id, job_kind, job_step) = match &record.owner {
                OwnerId::Job(jid) => {
                    let job = jobs.get(jid.as_str());
                    (
                        Some(jid.to_string()),
                        job.map(|j| j.kind.clone()),
                        job.map(|j| j.step.clone()),
                    )
                }
                OwnerId::Crew(_) => (None, None, None),
            };

            self.agent_meta.insert(
                record.agent_id.clone(),
                AgentMeta {
                    agent_kind: Some(record.agent_name.clone()),
                    job_id,
                    job_kind,
                    job_step,
                    project: Some(record.project.clone()),
                    status: format!("{}", record.status),
                },
            );

            // Query agent's usage via adapter
            let agent_id = oj_core::AgentId::from_string(&record.agent_id);
            if let Some(usage) = self.agents.fetch_usage(&agent_id).await {
                self.cached_usage.insert(record.agent_id.clone(), usage);
            }
        }

        // Build records and write
        let records = self.build_records();

        let write_result = if !records.is_empty() {
            self.rotate_if_needed();
            self.write_records(&records)
        } else {
            Ok(())
        };

        // Update health
        let now_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;

        let mut health = self.health.lock();
        health.last_collection_ms = now_ms;
        health.agents_tracked = self.cached_usage.len();
        health.ghost_agents = Vec::new();
        match write_result {
            Ok(()) => health.last_error = None,
            Err(e) => {
                tracing::warn!(error = %e, "metrics write failed");
                health.last_error = Some(e.to_string());
            }
        }
    }

    /// Build usage records from current cached state.
    fn build_records(&self) -> Vec<UsageRecord> {
        let now = format_utc_now();
        self.cached_usage
            .iter()
            .map(|(agent_id, usage)| {
                let meta = self.agent_meta.get(agent_id);
                UsageRecord {
                    timestamp: now.clone(),
                    agent_id: agent_id.clone(),
                    agent_kind: meta.and_then(|m| m.agent_kind.clone()),
                    job_id: meta.and_then(|m| m.job_id.clone()),
                    job_kind: meta.and_then(|m| m.job_kind.clone()),
                    job_step: meta.and_then(|m| m.job_step.clone()),
                    project: meta.and_then(|m| m.project.clone()),
                    status: meta.map(|m| m.status.clone()).unwrap_or("gone".to_string()),
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cache_creation_input_tokens: usage.cache_write_tokens,
                    cache_read_input_tokens: usage.cache_read_tokens,
                    total_cost_usd: if usage.total_cost_usd > 0.0 {
                        Some(usage.total_cost_usd)
                    } else {
                        None
                    },
                    total_api_ms: if usage.total_api_ms > 0 {
                        Some(usage.total_api_ms)
                    } else {
                        None
                    },
                }
            })
            .collect()
    }

    /// Append records to the JSONL file.
    fn write_records(&self, records: &[UsageRecord]) -> Result<(), std::io::Error> {
        let path = self.metrics_dir.join("usage.jsonl");
        fs::create_dir_all(&self.metrics_dir)?;

        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

        for record in records {
            if let Ok(line) = serde_json::to_string(record) {
                writeln!(file, "{}", line)?;
            }
        }
        file.sync_all()?;
        Ok(())
    }

    /// Rotate the metrics file if it exceeds the size limit.
    ///
    /// Before rotating, writes a final baseline of all in-memory records so
    /// the new file starts with complete data.
    fn rotate_if_needed(&self) {
        let path = self.metrics_dir.join("usage.jsonl");
        let size = match fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(_) => return,
        };

        if size < MAX_METRICS_SIZE {
            return;
        }

        let path_str = path.display().to_string();

        // Shift older rotations: .3 is deleted, .2→.3, .1→.2
        for i in (1..MAX_ROTATED_FILES).rev() {
            let from = format!("{path_str}.{i}");
            let to = format!("{path_str}.{}", i + 1);
            let _ = fs::rename(&from, &to);
        }

        // Rotate current → .1
        let _ = fs::rename(&path, format!("{path_str}.1"));

        // Write baseline to new file (all current in-memory records)
        let baseline = self.build_records();
        if let Err(e) = self.write_records(&baseline) {
            tracing::warn!(error = %e, "failed to write baseline after rotation");
        }
    }
}

#[cfg(test)]
#[path = "usage_metrics_tests.rs"]
mod tests;
