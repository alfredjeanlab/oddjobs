// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Append-only logger for per-queue activity logs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use oj_core::ShortId;

use crate::log_paths;

/// Append-only logger for per-queue activity logs.
///
/// Writes human-readable timestamped lines to:
///   `<log_dir>/queue/<queue_name>.log`
///
/// Each `append()` call opens, writes, and closes the file.
/// This is safe for the low write frequency of queue events.
pub struct QueueLogger {
    log_dir: PathBuf,
}

impl QueueLogger {
    pub fn new(log_dir: PathBuf) -> Self {
        Self { log_dir }
    }

    /// Append a timestamped log line to the queue's log file.
    ///
    /// Format: `2026-01-30T08:14:09Z [item_id_prefix] message`
    ///
    /// Failures are logged via tracing but do not propagate — logging
    /// must not break the engine.
    pub fn append(&self, queue_name: &str, item_id: &str, message: &str) {
        let path = log_paths::queue_log_path(&self.log_dir, queue_name);
        if let Err(e) = self.write_line(&path, item_id, message) {
            tracing::warn!(
                queue_name,
                error = %e,
                "failed to write queue log"
            );
        }
    }

    fn write_line(&self, path: &Path, item_id: &str, message: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let ts = format_utc_now();
        let prefix = item_id.short(8);
        writeln!(file, "{} [{}] {}", ts, prefix, message)?;
        Ok(())
    }
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
#[path = "queue_logger_tests.rs"]
mod tests;
