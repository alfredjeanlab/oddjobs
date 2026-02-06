// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron time display helpers for query responses.

use oj_storage::CronRecord;

/// Compute the human-readable time display for a cron.
///
/// Running crons show "in Xm" countdown to next fire.
/// Stopped crons show "Xm ago" since last fire, or "-" if never fired.
pub(super) fn cron_time_display(cron: &CronRecord, now_ms: u64) -> String {
    if cron.status == "running" {
        let base_ms = cron.last_fired_at_ms.unwrap_or(cron.started_at_ms);
        if base_ms == 0 {
            return "-".to_string();
        }
        let interval_ms = parse_interval_ms(&cron.interval).unwrap_or(0);
        let next_fire_ms = base_ms.saturating_add(interval_ms);
        if next_fire_ms <= now_ms {
            "now".to_string()
        } else {
            let remaining_secs = (next_fire_ms - now_ms) / 1000;
            format!("in {}", format_duration_short(remaining_secs))
        }
    } else {
        // stopped
        match cron.last_fired_at_ms {
            Some(fired_ms) if fired_ms > 0 && now_ms >= fired_ms => {
                let ago_secs = (now_ms - fired_ms) / 1000;
                format!("{} ago", format_duration_short(ago_secs))
            }
            _ => "-".to_string(),
        }
    }
}

/// Parse an interval string like "30m", "1h", "6h" into milliseconds.
pub(super) fn parse_interval_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_str, suffix) = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| (&s[..i], &s[i..]))
        .unwrap_or((s, ""));
    let num: u64 = num_str.parse().ok()?;
    let multiplier = match suffix.trim() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => 1_000,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600_000,
        "d" | "day" | "days" => 86_400_000,
        _ => return None,
    };
    Some(num * multiplier)
}

/// Format seconds as a short human-readable duration: "12s", "5m", "2h", "3d".
pub(super) fn format_duration_short(total_secs: u64) -> String {
    oj_core::format_elapsed(total_secs)
}

#[cfg(test)]
#[path = "query_crons_tests.rs"]
mod tests;
