// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::listener::query::query_crons::{
    cron_time_display, format_duration_short, parse_interval_ms,
};
use oj_storage::CronRecord;

fn make_cron_record(
    status: &str,
    interval: &str,
    started_at_ms: u64,
    last_fired_at_ms: Option<u64>,
) -> CronRecord {
    CronRecord {
        name: "test".to_string(),
        namespace: String::new(),
        project_root: std::path::PathBuf::from("/test"),
        runbook_hash: String::new(),
        status: status.to_string(),
        interval: interval.to_string(),
        run_target: "job:cleanup".to_string(),
        started_at_ms,
        last_fired_at_ms,
    }
}

const NOW: u64 = 100_000_000;

#[yare::parameterized(
    countdown_20m = { "running", "30m", NOW - 600_000, None, "in 20m" },
    after_fire_30m = { "running", "1h", NOW - 7_200_000, Some(NOW - 1_800_000), "in 30m" },
    overdue = { "running", "5m", NOW - 600_000, None, "now" },
    zero_started = { "running", "30m", 0, None, "-" },
    stopped_1h_ago = { "stopped", "30m", NOW - 7_200_000, Some(NOW - 3_600_000), "1h ago" },
    stopped_never = { "stopped", "30m", NOW - 600_000, None, "-" },
)]
fn time_display(status: &str, interval: &str, started: u64, fired: Option<u64>, expected: &str) {
    let cron = make_cron_record(status, interval, started, fired);
    assert_eq!(cron_time_display(&cron, NOW), expected);
}

#[yare::parameterized(
    seconds = { "30s", Some(30_000) },
    minutes = { "5m", Some(300_000) },
    hours = { "1h", Some(3_600_000) },
    days = { "2d", Some(172_800_000) },
    invalid = { "invalid", None },
)]
fn interval_parsing(input: &str, expected: Option<u64>) {
    assert_eq!(parse_interval_ms(input), expected);
}

#[yare::parameterized(
    zero = { 0, "0s" },
    seconds = { 45, "45s" },
    one_minute = { 60, "1m" },
    minutes = { 3599, "59m" },
    one_hour = { 3600, "1h" },
    hours = { 86399, "23h" },
    one_day = { 86400, "1d" },
    days = { 172800, "2d" },
)]
fn duration_formatting(secs: u64, expected: &str) {
    assert_eq!(format_duration_short(secs), expected);
}
