use serial_test::serial;

use super::super::{format_duration, format_text, friendly_name_label, truncate_reason};
use super::{empty_ns, job_entry, setup_no_color};

// ── format_duration ─────────────────────────────────────────────────

#[yare::parameterized(
    zero_seconds = { 0, "0s" },
    max_seconds = { 59, "59s" },
    one_minute = { 60, "1m" },
    max_minutes = { 3599, "59m" },
    one_hour = { 3600, "1h" },
    hour_and_minutes = { 3660, "1h1m" },
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
fn friendly_name_label_check(name: &str, kind: &str, id: &str, expected: &str) {
    assert_eq!(friendly_name_label(name, kind, id), expected);
}

// ── header line ─────────────────────────────────────────────────────

#[test]
#[serial]
fn header_without_watch_interval() {
    setup_no_color();
    let out = format_text(120, &[], None);
    assert_eq!(out, "oj daemon: running 2m\n");
}

#[test]
#[serial]
fn header_with_watch_interval() {
    setup_no_color();
    let out = format_text(120, &[], Some("5s"));
    assert_eq!(out, "oj daemon: running 2m | every 5s\n");
}

#[test]
#[serial]
fn header_with_custom_watch_interval() {
    setup_no_color();
    let out = format_text(3700, &[], Some("10s"));
    assert_eq!(out, "oj daemon: running 1h1m | every 10s\n");
}

#[test]
#[serial]
fn header_with_active_jobs_and_watch() {
    setup_no_color();

    let mut entry = job_entry("abc12345", "job", "compile");
    entry.name = "build".to_string();
    entry.elapsed_ms = 5000;
    let mut ns = empty_ns("test");
    ns.active_jobs.push(entry);

    let out = format_text(60, &[ns], Some("2s"));
    let first_line = out.lines().next().unwrap();
    assert_eq!(
        first_line,
        "oj daemon: running 1m | every 2s | 1 active job"
    );
}

#[test]
#[serial]
fn header_without_watch_has_no_every() {
    setup_no_color();

    let mut entry = job_entry("abc12345", "job", "compile");
    entry.name = "build".to_string();
    entry.elapsed_ms = 5000;
    let mut ns = empty_ns("test");
    ns.active_jobs.push(entry);

    let out = format_text(60, &[ns], None);
    let first_line = out.lines().next().unwrap();
    assert_eq!(first_line, "oj daemon: running 1m | 1 active job");
    assert!(!first_line.contains("every"));
}

// ── decisions pending ───────────────────────────────────────────────

#[test]
#[serial]
fn header_shows_decisions_pending_singular() {
    setup_no_color();

    let mut ns = empty_ns("test");
    ns.pending_decisions = 1;
    let out = format_text(60, &[ns], None);
    let first_line = out.lines().next().unwrap();
    assert!(
        first_line.contains("| 1 decision pending"),
        "header should show singular decision pending: {first_line}"
    );
    assert!(
        !first_line.contains("decisions"),
        "singular should not have trailing 's': {first_line}"
    );
}

#[test]
#[serial]
fn header_shows_decisions_pending_plural() {
    setup_no_color();

    let mut ns1 = empty_ns("proj-a");
    ns1.pending_decisions = 2;
    let mut ns2 = empty_ns("proj-b");
    ns2.pending_decisions = 1;
    let out = format_text(60, &[ns1, ns2], None);
    let first_line = out.lines().next().unwrap();
    assert!(
        first_line.contains("| 3 decisions pending"),
        "header should show total decisions pending across namespaces: {first_line}"
    );
}

#[test]
#[serial]
fn header_hides_decisions_when_zero() {
    setup_no_color();

    let ns = empty_ns("test");
    let out = format_text(60, &[ns], None);
    assert!(
        !out.contains("decision"),
        "header should not mention decisions when count is zero: {out}"
    );
}

// ── namespace visibility ────────────────────────────────────────────

#[test]
#[serial]
fn namespace_with_only_empty_queues_is_hidden() {
    setup_no_color();

    let mut ns = empty_ns("empty-project");
    ns.queues.push(oj_daemon::QueueStatus {
        name: "tasks".to_string(),
        pending: 0,
        active: 0,
        dead: 0,
    });

    let output = format_text(60, &[ns], None);
    assert!(
        !output.contains("empty-project"),
        "namespace with only empty queues should be hidden:\n{output}"
    );
    assert_eq!(output, "oj daemon: running 1m\n");
}

#[test]
#[serial]
fn namespace_with_non_empty_queue_is_shown() {
    setup_no_color();

    let mut ns = empty_ns("active-project");
    ns.queues.push(oj_daemon::QueueStatus {
        name: "tasks".to_string(),
        pending: 1,
        active: 0,
        dead: 0,
    });

    let output = format_text(60, &[ns], None);
    assert!(
        output.contains("active-project"),
        "namespace with non-empty queue should be shown:\n{output}"
    );
    assert!(
        output.contains("tasks"),
        "queue should be displayed:\n{output}"
    );
}
