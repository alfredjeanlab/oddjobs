// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared human-readable duration formatting.

/// Format seconds as a short human-readable duration: `"5s"`, `"2m"`, `"1h30m"`, `"3d"`.
///
/// For the hours range, minutes are included when non-zero (e.g. `"1h"` vs `"1h5m"`).
pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m > 0 {
            format!("{}h{}m", h, m)
        } else {
            format!("{}h", h)
        }
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Format milliseconds as a short human-readable duration.
///
/// Convenience wrapper around [`format_elapsed`].
pub fn format_elapsed_ms(ms: u64) -> String {
    format_elapsed(ms / 1000)
}

#[cfg(test)]
#[path = "time_fmt_tests.rs"]
mod tests;
