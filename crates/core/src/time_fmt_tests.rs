// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{format_elapsed, format_elapsed_ms};

#[yare::parameterized(
    zero_seconds     = { 0,      "0s" },
    max_seconds      = { 59,     "59s" },
    one_minute       = { 60,     "1m" },
    max_minutes      = { 3599,   "59m" },
    one_hour         = { 3600,   "1h" },
    hour_and_minutes = { 3660,   "1h1m" },
    hours_no_minutes = { 7200,   "2h" },
    almost_a_day     = { 86399,  "23h59m" },
    one_day          = { 86400,  "1d" },
    two_days         = { 172800, "2d" },
)]
fn elapsed(secs: u64, expected: &str) {
    assert_eq!(format_elapsed(secs), expected);
}

#[yare::parameterized(
    five_seconds = { 5_000,     "5s" },
    two_minutes  = { 120_000,   "2m" },
    one_hour     = { 3_600_000, "1h" },
)]
fn elapsed_ms(ms: u64, expected: &str) {
    assert_eq!(format_elapsed_ms(ms), expected);
}
