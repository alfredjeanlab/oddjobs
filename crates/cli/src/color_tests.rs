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
fn literal_produces_ansi_when_color_forced() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let result = literal("bar");
    assert!(
        result.contains("\x1b[38;5;250m"),
        "expected ANSI literal color"
    );
    assert!(result.contains("bar"));
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
    assert_eq!(literal("bar"), "bar");
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
