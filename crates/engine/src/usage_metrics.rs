// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent usage metrics collection.
//!
//! Periodically scans Claude's JSONL session logs for token usage data and
//! writes cumulative records to an append-only JSONL file at
//! `~/.local/state/oj/metrics/usage.jsonl`.
//!
//! The collector runs as a background tokio task (like `spawn_checkpoint`)
//! and writes frequently enough that cost data survives daemon crashes.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use oj_core::OwnerId;
use oj_storage::MaterializedState;

/// Default collection interval (30 seconds).
const DEFAULT_INTERVAL_SECS: u64 = 30;

/// Maximum metrics file size before rotation (10 MB).
const MAX_METRICS_SIZE: u64 = 10 * 1024 * 1024;

/// Number of rotated files to keep (usage.jsonl.1, .2, .3).
const MAX_ROTATED_FILES: u32 = 3;

/// Ghost detection runs every N collection cycles (~5 minutes at 30s interval).
const GHOST_CHECK_EVERY_N: u64 = 10;

/// A single usage record written to the JSONL metrics file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub timestamp: String,
    pub agent_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub status: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Health information shared with the listener for `oj status`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsHealth {
    pub last_collection_ms: u64,
    pub sessions_tracked: usize,
    pub last_error: Option<String>,
    pub ghost_sessions: Vec<String>,
}

/// Internal per-session parser state for incremental reading.
struct SessionParseState {
    offset: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    model: Option<String>,
}

/// Deltas returned from incremental parsing of a session log.
pub(crate) struct UsageDeltas {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub model: Option<String>,
}

/// Parse token usage from a Claude JSONL session log starting at `offset`.
///
/// Reads only `type: "assistant"` records with `message.usage` fields.
/// Returns deltas (token counts found in new records) and the new byte offset.
pub(crate) fn parse_session_usage(path: &Path, offset: u64) -> (UsageDeltas, u64) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => {
            return (
                UsageDeltas {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                    model: None,
                },
                offset,
            );
        }
    };

    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::Start(offset)).is_err() {
        return (
            UsageDeltas {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                model: None,
            },
            offset,
        );
    }

    let mut deltas = UsageDeltas {
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        model: None,
    };
    let mut current_offset = offset;
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(n) => {
                if !line.ends_with('\n') {
                    // Incomplete line — don't advance offset
                    break;
                }
                current_offset += n as u64;

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let json: serde_json::Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if json.get("type").and_then(|v| v.as_str()) != Some("assistant") {
                    continue;
                }

                let Some(message) = json.get("message") else {
                    continue;
                };

                if let Some(usage) = message.get("usage") {
                    deltas.input_tokens += usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    deltas.output_tokens += usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    deltas.cache_creation_input_tokens += usage
                        .get("cache_creation_input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    deltas.cache_read_input_tokens += usage
                        .get("cache_read_input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                }

                if let Some(model) = message.get("model").and_then(|v| v.as_str()) {
                    deltas.model = Some(model.to_string());
                }
            }
            Err(_) => break,
        }
    }

    (deltas, current_offset)
}

/// Background metrics collector.
pub struct UsageMetricsCollector {
    state: Arc<Mutex<MaterializedState>>,
    metrics_dir: PathBuf,
    sessions: HashMap<String, SessionParseState>,
    /// Metadata enrichment: agent_id -> (agent_kind, job_id, job_kind, job_step, namespace, status)
    agent_meta: HashMap<String, AgentMeta>,
    health: Arc<Mutex<MetricsHealth>>,
    cycle_count: u64,
}

struct AgentMeta {
    agent_kind: Option<String>,
    job_id: Option<String>,
    job_kind: Option<String>,
    job_step: Option<String>,
    namespace: Option<String>,
    status: String,
}

impl UsageMetricsCollector {
    /// Spawn the background metrics collector task.
    ///
    /// Returns a shared health handle for the listener to query.
    pub fn spawn_collector(
        state: Arc<Mutex<MaterializedState>>,
        metrics_dir: PathBuf,
    ) -> Arc<Mutex<MetricsHealth>> {
        let health = Arc::new(Mutex::new(MetricsHealth::default()));

        let mut collector = UsageMetricsCollector {
            state,
            metrics_dir,
            sessions: HashMap::new(),
            agent_meta: HashMap::new(),
            health: Arc::clone(&health),
            cycle_count: 0,
        };

        let interval_secs = std::env::var("OJ_METRICS_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_INTERVAL_SECS);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

            loop {
                interval.tick().await;
                collector.collect_once();
            }
        });

        health
    }

    /// Run one collection cycle: snapshot state, parse logs, write records.
    fn collect_once(&mut self) {
        self.cycle_count += 1;

        // Snapshot agents, jobs, agent_runs, and sessions from state (brief lock)
        let (agents, jobs, agent_runs, sessions) = {
            let state = self.state.lock();
            (
                state.agents.clone(),
                state.jobs.clone(),
                state.agent_runs.clone(),
                state.sessions.clone(),
            )
        };

        // Update metadata and discover session logs
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
                OwnerId::AgentRun(_) => (None, None, None),
            };

            self.agent_meta.insert(
                record.agent_id.clone(),
                AgentMeta {
                    agent_kind: Some(record.agent_name.clone()),
                    job_id,
                    job_kind,
                    job_step,
                    namespace: Some(record.namespace.clone()),
                    status: format!("{}", record.status),
                },
            );

            // Parse incremental usage from session log
            let session_log =
                oj_adapters::agent::find_session_log(&record.workspace_path, &record.agent_id);
            if let Some(log_path) = session_log {
                let parse_state =
                    self.sessions
                        .entry(record.agent_id.clone())
                        .or_insert_with(|| SessionParseState {
                            offset: 0,
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                            model: None,
                        });

                let (deltas, new_offset) = parse_session_usage(&log_path, parse_state.offset);
                parse_state.offset = new_offset;
                parse_state.input_tokens += deltas.input_tokens;
                parse_state.output_tokens += deltas.output_tokens;
                parse_state.cache_creation_input_tokens += deltas.cache_creation_input_tokens;
                parse_state.cache_read_input_tokens += deltas.cache_read_input_tokens;
                if deltas.model.is_some() {
                    parse_state.model = deltas.model;
                }
            }
        }

        // Ghost detection (every N cycles)
        let ghost_sessions = if self.cycle_count.is_multiple_of(GHOST_CHECK_EVERY_N) {
            detect_ghost_sessions(&agents, &agent_runs, &sessions)
        } else {
            // Preserve previous ghost list
            self.health.lock().ghost_sessions.clone()
        };

        // Build records and write
        let now = iso_now();
        let records: Vec<UsageRecord> = self
            .sessions
            .iter()
            .map(|(agent_id, parse)| {
                let meta = self.agent_meta.get(agent_id);
                UsageRecord {
                    timestamp: now.clone(),
                    agent_id: agent_id.clone(),
                    session_id: agent_id.clone(),
                    agent_kind: meta.and_then(|m| m.agent_kind.clone()),
                    job_id: meta.and_then(|m| m.job_id.clone()),
                    job_kind: meta.and_then(|m| m.job_kind.clone()),
                    job_step: meta.and_then(|m| m.job_step.clone()),
                    namespace: meta.and_then(|m| m.namespace.clone()),
                    status: meta.map(|m| m.status.clone()).unwrap_or("gone".to_string()),
                    input_tokens: parse.input_tokens,
                    output_tokens: parse.output_tokens,
                    cache_creation_input_tokens: parse.cache_creation_input_tokens,
                    cache_read_input_tokens: parse.cache_read_input_tokens,
                    model: parse.model.clone(),
                }
            })
            .collect();

        let write_result = if !records.is_empty() {
            self.rotate_if_needed();
            self.write_records(&records)
        } else {
            Ok(())
        };

        // Update health
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut health = self.health.lock();
        health.last_collection_ms = now_ms;
        health.sessions_tracked = self.sessions.len();
        health.ghost_sessions = ghost_sessions;
        match write_result {
            Ok(()) => health.last_error = None,
            Err(e) => {
                tracing::warn!(error = %e, "metrics write failed");
                health.last_error = Some(e.to_string());
            }
        }
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
        // Errors here are non-fatal — the new file will get populated on next cycle
        let now = iso_now();
        let baseline: Vec<UsageRecord> = self
            .sessions
            .iter()
            .map(|(agent_id, parse)| {
                let meta = self.agent_meta.get(agent_id);
                UsageRecord {
                    timestamp: now.clone(),
                    agent_id: agent_id.clone(),
                    session_id: agent_id.clone(),
                    agent_kind: meta.and_then(|m| m.agent_kind.clone()),
                    job_id: meta.and_then(|m| m.job_id.clone()),
                    job_kind: meta.and_then(|m| m.job_kind.clone()),
                    job_step: meta.and_then(|m| m.job_step.clone()),
                    namespace: meta.and_then(|m| m.namespace.clone()),
                    status: meta.map(|m| m.status.clone()).unwrap_or("gone".to_string()),
                    input_tokens: parse.input_tokens,
                    output_tokens: parse.output_tokens,
                    cache_creation_input_tokens: parse.cache_creation_input_tokens,
                    cache_read_input_tokens: parse.cache_read_input_tokens,
                    model: parse.model.clone(),
                }
            })
            .collect();

        if let Err(e) = self.write_records(&baseline) {
            tracing::warn!(error = %e, "failed to write baseline after rotation");
        }
    }
}

/// Detect tmux sessions with `oj-` prefix that are not tracked in state.
fn detect_ghost_sessions(
    agents: &HashMap<String, oj_core::AgentRecord>,
    agent_runs: &HashMap<String, oj_core::AgentRun>,
    sessions: &HashMap<String, oj_storage::Session>,
) -> Vec<String> {
    let output = match std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut known_sessions: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for r in agents.values() {
        if let Some(sid) = &r.session_id {
            known_sessions.insert(sid);
        }
    }
    for ar in agent_runs.values() {
        if let Some(sid) = &ar.session_id {
            known_sessions.insert(sid);
        }
    }
    for s in sessions.values() {
        known_sessions.insert(&s.id);
    }

    stdout
        .lines()
        .filter(|name| name.starts_with("oj-"))
        .filter(|name| !known_sessions.contains(name))
        .map(String::from)
        .collect()
}

/// Generate an ISO-8601 timestamp string.
fn iso_now() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();

    // Simple epoch → ISO conversion (UTC)
    let days = secs / 86400;
    let day_secs = secs % 86400;
    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let sec = day_secs % 60;

    // Convert days since epoch to Y-M-D (civil_from_days algorithm)
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

#[cfg(test)]
#[path = "usage_metrics_tests.rs"]
mod tests;
