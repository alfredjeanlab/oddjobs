use serial_test::serial;

use super::super::format_text;
use super::{empty_ns, job_entry, setup_no_color};

// ── name display ────────────────────────────────────────────────────

#[test]
#[serial]
fn active_job_shows_kind_not_name() {
    setup_no_color();

    let mut ns = empty_ns("myproject");
    ns.active_jobs
        .push(job_entry("abcd1234-0000-0000-0000", "build", "check"));

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("build"),
        "output should contain job kind 'build':\n{output}"
    );
    assert!(
        output.contains("check"),
        "output should contain step name 'check':\n{output}"
    );
    // The job UUID name should not appear in the rendered text
    assert!(
        !output.contains("abcd1234-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

#[test]
#[serial]
fn active_job_hides_nonce_only_name() {
    setup_no_color();

    let mut entry = job_entry("abcd1234-0000-0000-0000", "build", "check");
    entry.name = "abcd1234".to_string(); // nonce-only name (first 8 chars of ID)
    let mut ns = empty_ns("myproject");
    ns.active_jobs.push(entry);

    let output = format_text(30, &[ns], None);

    assert!(output.contains("build"));
    assert!(output.contains("check"));
    let nonce_count = output.matches("abcd1234").count();
    assert_eq!(
        nonce_count, 1,
        "nonce 'abcd1234' should appear exactly once (as truncated ID), not twice:\n{output}"
    );
}

#[test]
#[serial]
fn escalated_job_hides_name_when_same_as_id() {
    setup_no_color();

    let mut entry = job_entry("efgh5678-0000-0000-0000", "deploy", "test");
    entry.step_status = "waiting".to_string();
    entry.waiting_reason = Some("gate check failed".to_string());
    let mut ns = empty_ns("myproject");
    ns.escalated_jobs.push(entry);

    let output = format_text(30, &[ns], None);

    assert!(output.contains("deploy"));
    assert!(output.contains("test"));
    assert!(output.contains("gate check failed"));
    assert!(
        !output.contains("efgh5678-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

#[test]
#[serial]
fn orphaned_job_hides_name_when_same_as_id() {
    setup_no_color();

    let mut ns = empty_ns("myproject");
    ns.orphaned_jobs
        .push(job_entry("ijkl9012-0000-0000-0000", "ci", "lint"));

    let output = format_text(30, &[ns], None);

    assert!(output.contains("ci"));
    assert!(output.contains("lint"));
    assert!(
        !output.contains("ijkl9012-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

// ── friendly name display ───────────────────────────────────────────

#[test]
#[serial]
fn active_job_shows_friendly_name() {
    setup_no_color();

    let mut entry = job_entry("abcd1234-0000-0000-0000", "build", "check");
    entry.name = "fix-login-button-abcd1234".to_string();
    let mut ns = empty_ns("myproject");
    ns.active_jobs.push(entry);

    let output = format_text(30, &[ns], None);

    assert!(output.contains("build"));
    assert!(
        output.contains("fix-login-button-abcd1234"),
        "output should contain friendly name:\n{output}"
    );
    assert!(output.contains("check"));
}

#[test]
#[serial]
fn escalated_job_shows_friendly_name() {
    setup_no_color();

    let mut entry = job_entry("efgh5678-0000-0000-0000", "deploy", "test");
    entry.name = "deploy-staging-efgh5678".to_string();
    entry.step_status = "waiting".to_string();
    entry.waiting_reason = Some("gate check failed".to_string());
    let mut ns = empty_ns("myproject");
    ns.escalated_jobs.push(entry);

    let output = format_text(30, &[ns], None);

    assert!(output.contains("deploy"));
    assert!(
        output.contains("deploy-staging-efgh5678"),
        "output should contain friendly name:\n{output}"
    );
}

#[test]
#[serial]
fn orphaned_job_shows_friendly_name() {
    setup_no_color();

    let mut entry = job_entry("ijkl9012-0000-0000-0000", "ci", "lint");
    entry.name = "ci-main-branch-ijkl9012".to_string();
    let mut ns = empty_ns("myproject");
    ns.orphaned_jobs.push(entry);

    let output = format_text(30, &[ns], None);

    assert!(output.contains("ci"));
    assert!(
        output.contains("ci-main-branch-ijkl9012"),
        "output should contain friendly name:\n{output}"
    );
}

// ── escalate source label ───────────────────────────────────────────

#[test]
#[serial]
fn escalated_job_shows_source_label() {
    setup_no_color();

    let mut entry = job_entry("efgh5678-0000-0000-0000", "deploy", "test");
    entry.name = "deploy-staging-efgh5678".to_string();
    entry.step_status = "waiting".to_string();
    entry.waiting_reason = Some("Agent is idle".to_string());
    entry.escalate_source = Some("idle".to_string());
    let mut ns = empty_ns("myproject");
    ns.escalated_jobs.push(entry);

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("[idle]"),
        "output should contain source label '[idle]':\n{output}"
    );
}

#[test]
#[serial]
fn escalated_job_no_source_label_when_none() {
    setup_no_color();

    let mut entry = job_entry("efgh5678-0000-0000-0000", "deploy", "test");
    entry.name = "deploy-staging-efgh5678".to_string();
    entry.step_status = "waiting".to_string();
    entry.waiting_reason = Some("gate check failed".to_string());
    let mut ns = empty_ns("myproject");
    ns.escalated_jobs.push(entry);

    let output = format_text(30, &[ns], None);

    assert!(
        !output.contains('['),
        "output should not contain bracket source label when source is None:\n{output}"
    );
}

// ── truncated reason in output ──────────────────────────────────────

#[test]
#[serial]
fn escalated_job_truncates_long_reason() {
    setup_no_color();

    let long_reason = "e".repeat(200);
    let mut entry = job_entry("efgh5678", "deploy", "test");
    entry.name = "efgh5678".to_string();
    entry.step_status = "Waiting".to_string();
    entry.waiting_reason = Some(long_reason.clone());
    let mut ns = empty_ns("myproject");
    ns.escalated_jobs.push(entry);

    let output = format_text(30, &[ns], None);

    assert!(
        !output.contains(&long_reason),
        "output should not contain the full long reason:\n{output}"
    );
    assert!(
        output.contains("..."),
        "output should contain truncation indicator '...':\n{output}"
    );
}
