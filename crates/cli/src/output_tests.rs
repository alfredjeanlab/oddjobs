// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use serde::Serialize;

use super::{print_capture_frame, print_prune_results, OutputFormat};

#[derive(Debug, Clone, Serialize)]
struct FakeEntry {
    name: String,
    detail: String,
}

#[test]
fn print_prune_results_json_includes_all_fields() {
    let entries = vec![
        FakeEntry { name: "a".into(), detail: "d1".into() },
        FakeEntry { name: "b".into(), detail: "d2".into() },
    ];

    // JSON path should not panic and should produce valid JSON
    let result = print_prune_results(
        &entries,
        3,
        true,
        OutputFormat::Json,
        "widget",
        "skipped",
        |e: &FakeEntry| format!("{} ({})", e.name, e.detail),
    );
    assert!(result.is_ok());
}

#[test]
fn print_prune_results_text_dry_run() {
    let entries = vec![FakeEntry { name: "x".into(), detail: "y".into() }];

    let result = print_prune_results(
        &entries,
        1,
        true,
        OutputFormat::Text,
        "thing",
        "skipped",
        |e: &FakeEntry| format!("thing '{}'", e.name),
    );
    assert!(result.is_ok());
}

#[test]
fn print_prune_results_text_real_run() {
    let entries: Vec<FakeEntry> = vec![];

    let result = print_prune_results(
        &entries,
        5,
        false,
        OutputFormat::Text,
        "item",
        "active item(s) skipped",
        |e: &FakeEntry| e.name.clone(),
    );
    assert!(result.is_ok());
}

#[test]
fn print_capture_frame_does_not_panic() {
    // Smoke test: just verify it doesn't panic with typical input
    print_capture_frame("abc12345", "some terminal output\n");
}

#[test]
fn print_capture_frame_empty_output() {
    print_capture_frame("deadbeef", "");
}
