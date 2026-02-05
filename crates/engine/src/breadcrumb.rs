// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job breadcrumb files for orphan detection.
//!
//! Breadcrumbs are write-only during normal operation. They capture a snapshot
//! of job state on creation and each step transition, written as
//! `<job-id>.crumb.json` alongside job log files.
//!
//! On daemon startup, breadcrumbs are scanned and cross-referenced with
//! recovered WAL/snapshot state to detect orphaned jobs.

use crate::log_paths;
use oj_core::Job;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Breadcrumb snapshot written to disk on job creation and step transitions.
///
/// Write-only during normal operation; read-only during orphan detection at startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breadcrumb {
    pub job_id: String,
    pub project: String,
    pub kind: String,
    pub name: String,
    pub vars: HashMap<String, String>,
    pub current_step: String,
    pub step_status: String,
    pub agents: Vec<BreadcrumbAgent>,
    pub workspace_id: Option<String>,
    pub workspace_root: Option<PathBuf>,
    pub updated_at: String,
    /// Content hash of the stored runbook (for resume from orphan state).
    #[serde(default)]
    pub runbook_hash: String,
    /// Working directory where commands execute.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

/// Agent information captured in a breadcrumb.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreadcrumbAgent {
    pub agent_id: String,
    pub session_name: Option<String>,
    pub log_path: PathBuf,
}

/// Writes breadcrumb files alongside job logs.
///
/// Each write atomically replaces the previous breadcrumb for that job.
/// Failures are logged via tracing but never propagate — breadcrumbs must not
/// break the engine.
pub struct BreadcrumbWriter {
    logs_dir: PathBuf,
}

impl BreadcrumbWriter {
    pub fn new(logs_dir: PathBuf) -> Self {
        Self { logs_dir }
    }

    /// Write a breadcrumb snapshot for the given job.
    pub fn write(&self, job: &Job) {
        let breadcrumb = self.build_breadcrumb(job);
        let path = log_paths::breadcrumb_path(&self.logs_dir, &breadcrumb.job_id);
        let tmp_path = path.with_extension("crumb.tmp");

        if let Err(e) = std::fs::create_dir_all(&self.logs_dir).and_then(|_| {
            let json = serde_json::to_string_pretty(&breadcrumb).map_err(std::io::Error::other)?;
            std::fs::write(&tmp_path, json.as_bytes())?;
            std::fs::rename(&tmp_path, &path)
        }) {
            tracing::warn!(
                job_id = %breadcrumb.job_id,
                error = %e,
                "failed to write breadcrumb"
            );
        }
    }

    /// Delete the breadcrumb file for a terminal job.
    pub fn delete(&self, job_id: &str) {
        let path = log_paths::breadcrumb_path(&self.logs_dir, job_id);
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(
                    job_id,
                    error = %e,
                    "failed to delete breadcrumb"
                );
            }
        }
    }

    fn build_breadcrumb(&self, job: &Job) -> Breadcrumb {
        let mut agents = Vec::new();

        // Collect agents from step history
        for record in &job.step_history {
            if let Some(ref agent_id) = record.agent_id {
                let is_current_step = record.name == job.step;
                agents.push(BreadcrumbAgent {
                    agent_id: agent_id.clone(),
                    session_name: if is_current_step {
                        job.session_id.clone()
                    } else {
                        None
                    },
                    log_path: log_paths::agent_log_path(&self.logs_dir, agent_id),
                });
            }
        }

        Breadcrumb {
            job_id: job.id.clone(),
            project: job.namespace.clone(),
            kind: job.kind.clone(),
            name: job.name.clone(),
            vars: job.vars.clone(),
            current_step: job.step.clone(),
            step_status: job.step_status.to_string(),
            agents,
            workspace_id: job.workspace_id.as_ref().map(|w| w.to_string()),
            workspace_root: job.workspace_path.clone(),
            updated_at: format_utc_now(),
            runbook_hash: job.runbook_hash.clone(),
            cwd: Some(job.cwd.clone()),
        }
    }
}

/// Scan the logs directory for breadcrumb files and return deserialized breadcrumbs.
///
/// Skips files that fail to parse (logs a warning).
pub fn scan_breadcrumbs(logs_dir: &Path) -> Vec<Breadcrumb> {
    let mut breadcrumbs = Vec::new();

    let entries = match std::fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        Err(_) => return breadcrumbs,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if !name.ends_with(".crumb.json") {
            continue;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Breadcrumb>(&content) {
                Ok(breadcrumb) => breadcrumbs.push(breadcrumb),
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "skipping corrupt breadcrumb file"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to read breadcrumb file"
                );
            }
        }
    }

    breadcrumbs
}

/// Format the current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
fn format_utc_now() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();

    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let (year, month, day) = days_to_civil(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to (year, month, day).
///
/// Algorithm from Howard Hinnant's `civil_from_days`.
fn days_to_civil(days: u64) -> (i64, u32, u32) {
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
    (y, m as u32, d as u32)
}

#[cfg(test)]
#[path = "breadcrumb_tests.rs"]
mod tests;
