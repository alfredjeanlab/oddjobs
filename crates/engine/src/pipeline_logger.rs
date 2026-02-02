// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Append-only logger for per-pipeline activity logs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::log_paths;

/// Append-only logger for per-pipeline activity logs.
///
/// Writes human-readable timestamped lines to:
///   `<log_dir>/<pipeline_id>.log`
///
/// Each `append()` call opens, writes, and closes the file.
/// This is safe for the low write frequency of pipeline events.
pub struct PipelineLogger {
    log_dir: PathBuf,
}

impl PipelineLogger {
    pub fn new(log_dir: PathBuf) -> Self {
        Self { log_dir }
    }

    /// Append a log line for the given pipeline.
    ///
    /// Format: `2026-01-30T08:14:09Z [step] message`
    ///
    /// Failures are logged via tracing but do not propagate — logging
    /// must not break the engine.
    pub fn append(&self, pipeline_id: &str, step: &str, message: &str) {
        let path = self.log_dir.join(format!("{}.log", pipeline_id));
        if let Err(e) = self.write_line(&path, step, message) {
            tracing::warn!(
                pipeline_id,
                error = %e,
                "failed to write pipeline log"
            );
        }
    }

    /// Append a pointer line to the agent log for a step.
    ///
    /// Format: `2026-01-30T08:17:00Z [step] agent log: /full/path/to/logs/agent/<agent_id>.log`
    pub fn append_agent_pointer(&self, pipeline_id: &str, step: &str, agent_id: &str) {
        let log_path = log_paths::agent_log_path(&self.log_dir, agent_id);
        let message = format!("agent log: {}", log_path.display());
        self.append(pipeline_id, step, &message);
    }

    /// Copy the agent's session.jsonl to the logs directory.
    ///
    /// Copies the source file to `{logs_dir}/agent/{agent_id}/session.jsonl`.
    /// Failures are logged via tracing but do not propagate.
    pub fn copy_session_log(&self, agent_id: &str, source: &Path) {
        let dest_dir = log_paths::agent_session_log_dir(&self.log_dir, agent_id);
        let dest = dest_dir.join("session.jsonl");

        if let Err(e) = fs::create_dir_all(&dest_dir) {
            tracing::warn!(
                agent_id,
                error = %e,
                "failed to create session log directory"
            );
            return;
        }

        if let Err(e) = fs::copy(source, &dest) {
            tracing::warn!(
                agent_id,
                source = %source.display(),
                dest = %dest.display(),
                error = %e,
                "failed to copy session log"
            );
        } else {
            tracing::debug!(
                agent_id,
                dest = %dest.display(),
                "copied session log"
            );
        }
    }

    fn write_line(&self, path: &Path, step: &str, message: &str) -> std::io::Result<()> {
        fs::create_dir_all(&self.log_dir)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let ts = format_utc_now();
        writeln!(file, "{} [{}] {}", ts, step, message)?;
        Ok(())
    }
}

/// Format the current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
fn format_utc_now() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();

    // Convert epoch seconds to date/time components
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Convert days since epoch to y/m/d (civil date from days)
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
    let doe = (z - era * 146097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

#[cfg(test)]
#[path = "pipeline_logger_tests.rs"]
mod tests;
