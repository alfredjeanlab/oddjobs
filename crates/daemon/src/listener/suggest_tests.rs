// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[yare::parameterized(
    identical    = { "foo",   "foo",   0 },
    substitution = { "mergeq", "merges", 1 },
    insertion    = { "merg",  "merge", 1 },
    deletion     = { "merge", "merg",  1 },
    both_empty   = { "",      "",      0 },
    left_empty   = { "",      "abc",   3 },
    right_empty  = { "abc",   "",      3 },
)]
fn edit_dist(a: &str, b: &str, expected: usize) {
    assert_eq!(edit_distance(a, b), expected);
}

#[test]
fn find_similar_returns_close_matches() {
    let candidates = vec!["merges", "deploys", "builds", "merge-queue"];
    let result = find_similar("mergeq", &candidates);
    assert!(result.contains(&"merges".to_string()));
}

#[test]
fn find_similar_returns_empty_for_no_match() {
    let candidates = vec!["deploys", "builds"];
    let result = find_similar("xyz", &candidates);
    assert!(result.is_empty());
}

#[test]
fn find_similar_includes_prefix_matches() {
    let candidates = vec!["merge-queue", "deploys"];
    let result = find_similar("merge", &candidates);
    assert!(result.contains(&"merge-queue".to_string()));
}

#[test]
fn find_similar_excludes_self() {
    let candidates = vec!["merges", "merges"];
    let result = find_similar("merges", &candidates);
    assert!(result.is_empty());
}

#[test]
fn find_similar_sorted_by_distance() {
    let candidates = vec!["deploys", "merges", "merge-queue"];
    let result = find_similar("mergeq", &candidates);
    // "merges" (distance 1) should come before "merge-queue" (prefix match, distance > 1)
    assert_eq!(result[0], "merges");
}

#[test]
fn format_suggestion_single() {
    let similar = vec!["merges".to_string()];
    assert_eq!(format_suggestion(&similar), "\n\n  did you mean: merges?");
}

#[test]
fn format_suggestion_multiple() {
    let similar = vec!["merges".to_string(), "merge-queue".to_string()];
    assert_eq!(format_suggestion(&similar), "\n\n  did you mean one of: merges, merge-queue?");
}

#[test]
fn format_suggestion_empty() {
    let similar: Vec<String> = vec![];
    assert_eq!(format_suggestion(&similar), "");
}

#[test]
fn format_cross_project_suggestion_formats_correctly() {
    let result = format_cross_project_suggestion("oj worker stop", "fix", "oddjobs");
    assert_eq!(result, "\n\n  did you mean: oj worker stop fix --project oddjobs?");
}
