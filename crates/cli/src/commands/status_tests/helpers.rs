use super::*;

// ── format_duration ─────────────────────────────────────────────────

#[yare::parameterized(
    zero_seconds = { 0, "0s" },
    fifty_nine_seconds = { 59, "59s" },
    one_minute = { 60, "1m" },
    fifty_nine_minutes = { 3599, "59m" },
    one_hour = { 3600, "1h" },
    one_hour_one_minute = { 3660, "1h1m" },
    one_day = { 86400, "1d" },
)]
fn format_duration_values(secs: u64, expected: &str) {
    assert_eq!(format_duration(secs), expected);
}

// ── truncate_reason ─────────────────────────────────────────────────

#[test]
fn truncate_reason_short_unchanged() {
    assert_eq!(
        truncate_reason("gate check failed", 72),
        "gate check failed"
    );
}

#[test]
fn truncate_reason_long_single_line() {
    let long = "a".repeat(100);
    let result = truncate_reason(&long, 72);
    assert_eq!(result.len(), 72);
    assert!(result.ends_with("..."));
    assert_eq!(result, format!("{}...", "a".repeat(69)));
}

#[test]
fn truncate_reason_multiline_takes_first_line() {
    let reason = "first line\nsecond line\nthird line";
    let result = truncate_reason(reason, 72);
    assert_eq!(result, "first line...");
    assert!(!result.contains("second"));
}

#[test]
fn truncate_reason_multiline_long_first_line() {
    let first_line = "x".repeat(100);
    let reason = format!("{}\nsecond line", first_line);
    let result = truncate_reason(&reason, 72);
    assert_eq!(result.len(), 72);
    assert!(result.ends_with("..."));
    assert_eq!(result, format!("{}...", "x".repeat(69)));
}

// ── friendly_name_label ─────────────────────────────────────────────

#[yare::parameterized(
    empty_when_name_equals_kind = { "build", "build", "abc123", "" },
    empty_when_name_equals_id = { "abc123", "build", "abc123", "" },
    empty_when_name_equals_truncated_id = { "abcd1234", "build", "abcd1234-0000-0000-0000", "" },
    empty_when_name_is_empty = { "", "build", "abc123", "" },
    shown_when_meaningful = { "fix-login-button-a1b2c3d4", "build", "abc123", "fix-login-button-a1b2c3d4" },
)]
fn friendly_name_label_cases(name: &str, kind: &str, id: &str, expected: &str) {
    assert_eq!(friendly_name_label(name, kind, id), expected);
}
