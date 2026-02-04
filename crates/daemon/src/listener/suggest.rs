// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! "Did you mean?" suggestion helpers for resource-lookup error messages.

use std::sync::Arc;

use parking_lot::Mutex;

use oj_storage::MaterializedState;

/// Levenshtein edit distance between two strings.
pub(super) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, val) in dp[0].iter_mut().enumerate() {
        *val = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[a.len()][b.len()]
}

/// Find similar names from a list of candidates.
/// Returns names within edit distance <= max(2, input.len()/3),
/// sorted by distance (closest first). Also includes prefix matches.
pub(super) fn find_similar(input: &str, candidates: &[&str]) -> Vec<String> {
    let threshold = (input.len() / 3).max(2);
    let mut matches: Vec<(usize, String)> = candidates
        .iter()
        .filter(|c| **c != input)
        .filter_map(|c| {
            let dist = edit_distance(input, c);
            if dist <= threshold || c.starts_with(input) || input.starts_with(c) {
                Some((dist, c.to_string()))
            } else {
                None
            }
        })
        .collect();
    matches.sort_by_key(|(d, _)| *d);
    matches.into_iter().map(|(_, name)| name).collect()
}

/// Format a "did you mean" hint for appending to an error message.
/// Returns empty string if no suggestions.
pub(super) fn format_suggestion(similar: &[String]) -> String {
    match similar.len() {
        0 => String::new(),
        1 => format!("\n\n  did you mean: {}?", similar[0]),
        _ => format!("\n\n  did you mean one of: {}?", similar.join(", ")),
    }
}

/// Parse a scoped key like "namespace/name" into (namespace, name).
/// Returns ("", key) when no slash is present.
pub(super) fn parse_scoped_key(scoped_key: &str) -> (String, String) {
    if let Some(pos) = scoped_key.find('/') {
        (
            scoped_key[..pos].to_string(),
            scoped_key[pos + 1..].to_string(),
        )
    } else {
        (String::new(), scoped_key.to_string())
    }
}

/// Resource type for cross-namespace lookups.
pub(super) enum ResourceType {
    Queue,
    Worker,
    Cron,
    Command,
}

/// Check if a resource name exists in another namespace's active state.
/// Returns the namespace name if found.
pub(super) fn find_in_other_namespaces(
    resource_type: ResourceType,
    name: &str,
    current_namespace: &str,
    state: &MaterializedState,
) -> Option<String> {
    match resource_type {
        ResourceType::Queue => state
            .queue_items
            .keys()
            .filter_map(|k| {
                let (ns, qname) = parse_scoped_key(k);
                if qname == name && ns != current_namespace {
                    Some(ns)
                } else {
                    None
                }
            })
            .next(),
        ResourceType::Worker => state
            .workers
            .values()
            .find(|w| w.name == name && w.namespace != current_namespace)
            .map(|w| w.namespace.clone()),
        ResourceType::Cron => state
            .crons
            .values()
            .find(|c| c.name == name && c.namespace != current_namespace)
            .map(|c| c.namespace.clone()),
        // Commands are stateless definitions; no cross-namespace tracking.
        ResourceType::Command => None,
    }
}

/// Format a cross-project suggestion.
/// E.g.: "\n\n  did you mean: oj worker stop fix --project oddjobs?"
pub(super) fn format_cross_project_suggestion(
    command_prefix: &str,
    name: &str,
    namespace: &str,
) -> String {
    format!(
        "\n\n  did you mean: {} {} --project {}?",
        command_prefix, name, namespace
    )
}

/// Generate a "did you mean" suggestion for a resource name.
///
/// Three-phase lookup:
/// 1. Fuzzy-match against names from runbooks on disk.
/// 2. Fuzzy-match against names from daemon state (current namespace).
/// 3. Cross-namespace exact match (suggests `--project`).
///
/// The two closures let callers specify how to collect candidate names:
/// - `collect_runbook_names()` → names from `.oj/runbooks/` (caller captures project root)
/// - `collect_state_names(state)` → names from materialized state
pub(super) fn suggest_for_resource(
    name: &str,
    namespace: &str,
    command_prefix: &str,
    state: &Arc<Mutex<MaterializedState>>,
    resource_type: ResourceType,
    collect_runbook_names: impl FnOnce() -> Vec<String>,
    collect_state_names: impl FnOnce(&MaterializedState) -> Vec<String>,
) -> String {
    // 1. Collect from runbooks
    {
        let names = collect_runbook_names();
        let candidates: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let similar = find_similar(name, &candidates);
        if !similar.is_empty() {
            return format_suggestion(&similar);
        }
    }

    // 2. Try suggestions from daemon state
    {
        let state = state.lock();
        let names = collect_state_names(&state);
        let candidates: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let similar = find_similar(name, &candidates);
        if !similar.is_empty() {
            return format_suggestion(&similar);
        }
    }

    // 3. Check for wrong project (cross-namespace)
    let state = state.lock();
    if let Some(other_ns) = find_in_other_namespaces(resource_type, name, namespace, &state) {
        return format_cross_project_suggestion(command_prefix, name, &other_ns);
    }

    String::new()
}

/// Generate a "did you mean" suggestion from pre-collected candidates.
///
/// Use this variant when the state lock is already held (e.g., in query handlers).
pub(super) fn suggest_from_candidates(
    name: &str,
    namespace: &str,
    command_prefix: &str,
    state: &MaterializedState,
    resource_type: ResourceType,
    candidates: &[String],
) -> String {
    let candidate_refs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let similar = find_similar(name, &candidate_refs);
    let hint = format_suggestion(&similar);

    if !hint.is_empty() {
        return hint;
    }

    if let Some(other_ns) = find_in_other_namespaces(resource_type, name, namespace, state) {
        return format_cross_project_suggestion(command_prefix, name, &other_ns);
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_identical() {
        assert_eq!(edit_distance("foo", "foo"), 0);
    }

    #[test]
    fn edit_distance_one_substitution() {
        assert_eq!(edit_distance("mergeq", "merges"), 1);
    }

    #[test]
    fn edit_distance_insertion() {
        assert_eq!(edit_distance("merg", "merge"), 1);
    }

    #[test]
    fn edit_distance_deletion() {
        assert_eq!(edit_distance("merge", "merg"), 1);
    }

    #[test]
    fn edit_distance_empty_strings() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("", "abc"), 3);
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
        assert_eq!(
            format_suggestion(&similar),
            "\n\n  did you mean one of: merges, merge-queue?"
        );
    }

    #[test]
    fn format_suggestion_empty() {
        let similar: Vec<String> = vec![];
        assert_eq!(format_suggestion(&similar), "");
    }

    #[test]
    fn format_cross_project_suggestion_formats_correctly() {
        let result = format_cross_project_suggestion("oj worker stop", "fix", "oddjobs");
        assert_eq!(
            result,
            "\n\n  did you mean: oj worker stop fix --project oddjobs?"
        );
    }
}
