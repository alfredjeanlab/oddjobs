// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use serial_test::serial;

#[test]
fn codes_have_expected_values() {
    assert_eq!(codes::HEADER, 74);
    assert_eq!(codes::LITERAL, 250);
    assert_eq!(codes::CONTEXT, 245);
    assert_eq!(codes::MUTED, 240);
}

#[test]
#[serial]
fn styles_returns_styled_when_color_forced() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let s = styles();
    let debug = format!("{:?}", s);
    assert_ne!(
        debug,
        format!("{:?}", clap::builder::styling::Styles::plain())
    );
}

#[test]
#[serial]
fn styles_returns_plain_when_no_color() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let s = styles();
    let debug = format!("{:?}", s);
    assert_eq!(
        debug,
        format!("{:?}", clap::builder::styling::Styles::plain())
    );
}

#[test]
#[serial]
fn header_produces_ansi_when_color_forced() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = header("foo");
    assert!(
        result.contains("\x1b[38;5;74m"),
        "expected ANSI header color"
    );
    assert!(result.contains("foo"));
    assert!(result.contains("\x1b[0m"), "expected ANSI reset");
}

#[test]
#[serial]
fn context_produces_ansi_when_color_forced() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = context("baz");
    assert!(
        result.contains("\x1b[38;5;245m"),
        "expected ANSI context color"
    );
}

#[test]
#[serial]
fn muted_produces_ansi_when_color_forced() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = muted("dim");
    assert!(
        result.contains("\x1b[38;5;240m"),
        "expected ANSI muted color"
    );
}

#[test]
#[serial]
fn helpers_plain_when_no_color() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    assert_eq!(header("foo"), "foo");
    assert_eq!(context("baz"), "baz");
    assert_eq!(muted("dim"), "dim");
}

#[test]
#[serial]
fn should_colorize_respects_no_color() {
    std::env::set_var("NO_COLOR", "1");
    std::env::set_var("COLOR", "1");
    assert!(!should_colorize(), "NO_COLOR=1 should override COLOR=1");
}

#[test]
#[serial]
fn should_colorize_respects_color_force() {
    std::env::remove_var("NO_COLOR");
    std::env::set_var("COLOR", "1");
    assert!(should_colorize(), "COLOR=1 should force color on");
}

#[test]
#[serial]
fn status_green_for_running() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = status("running");
    assert!(
        result.contains("\x1b[32m"),
        "expected green ANSI for running"
    );
    assert!(result.contains("running"));
    assert!(result.contains("\x1b[0m"), "expected ANSI reset");
}

#[test]
#[serial]
fn status_green_for_completed() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = status("completed");
    assert!(
        result.contains("\x1b[32m"),
        "expected green ANSI for completed"
    );
}

#[test]
#[serial]
fn status_yellow_for_waiting() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = status("waiting");
    assert!(
        result.contains("\x1b[33m"),
        "expected yellow ANSI for waiting"
    );
    assert!(result.contains("waiting"));
}

#[test]
#[serial]
fn status_yellow_for_escalated() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = status("escalated");
    assert!(
        result.contains("\x1b[33m"),
        "expected yellow ANSI for escalated"
    );
}

#[test]
#[serial]
fn status_red_for_failed() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = status("failed");
    assert!(result.contains("\x1b[31m"), "expected red ANSI for failed");
    assert!(result.contains("failed"));
}

#[test]
#[serial]
fn status_red_for_cancelled() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = status("cancelled");
    assert!(
        result.contains("\x1b[31m"),
        "expected red ANSI for cancelled"
    );
}

#[test]
#[serial]
fn status_plain_when_no_color() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    assert_eq!(status("running"), "running");
    assert_eq!(status("failed"), "failed");
    assert_eq!(status("waiting"), "waiting");
}

#[test]
#[serial]
fn status_unknown_returns_plain() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = status("custom_status");
    assert_eq!(
        result, "custom_status",
        "unknown statuses should not be colored"
    );
}

#[test]
#[serial]
fn status_case_insensitive() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = status("Running");
    assert!(
        result.contains("\x1b[32m"),
        "expected green ANSI for Running (case insensitive)"
    );
    assert!(
        result.contains("Running"),
        "should preserve original casing"
    );
}
