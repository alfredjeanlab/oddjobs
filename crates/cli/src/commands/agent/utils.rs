// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Pure utility functions for agent commands.

use std::io::Write;
use std::path::PathBuf;

use oj_core::PromptType;

/// Resolve the OJ state directory from environment or default.
pub(super) fn get_state_dir() -> PathBuf {
    crate::env::state_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Format current UTC time as an ISO 8601 timestamp.
pub(super) fn utc_timestamp() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Convert epoch seconds to date-time components
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since epoch to Y-M-D (civil calendar from days)
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Append a timestamped line to the agent log file.
/// Failures are silently ignored since logging should not block the hook.
pub(super) fn append_agent_log(agent_id: &str, message: &str) {
    let log_path = get_state_dir()
        .join("logs/agent")
        .join(format!("{agent_id}.log"));
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let timestamp = utc_timestamp();
    let line = format!("{timestamp} stop-hook: {message}\n");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Map a tool name from PreToolUse input to its corresponding PromptType.
/// Returns None for unrecognized tools.
pub(super) fn prompt_type_for_tool(tool_name: Option<&str>) -> Option<PromptType> {
    match tool_name {
        Some("ExitPlanMode") => Some(PromptType::PlanApproval),
        Some("AskUserQuestion") => Some(PromptType::Question),
        _ => None,
    }
}

#[cfg(test)]
#[path = "utils_tests.rs"]
mod tests;
