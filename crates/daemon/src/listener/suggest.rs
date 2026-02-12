// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! "Did you mean?" suggestion helpers for resource-lookup error messages.

use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::split_scoped_name;
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
            dp[i][j] = (dp[i - 1][j] + 1).min(dp[i][j - 1] + 1).min(dp[i - 1][j - 1] + cost);
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

/// Resource type for cross-project lookups.
pub(super) enum ResourceType {
    Queue,
    Worker,
    Cron,
    Command,
}

/// Check if a resource name exists in another project's active state.
/// Returns the project name if found.
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
                let (ns, qname) = split_scoped_name(k);
                if qname == name && ns != current_namespace {
                    Some(ns.to_string())
                } else {
                    None
                }
            })
            .next(),
        ResourceType::Worker => state
            .workers
            .values()
            .find(|w| w.name == name && w.project != current_namespace)
            .map(|w| w.project.clone()),
        ResourceType::Cron => state
            .crons
            .values()
            .find(|c| c.name == name && c.project != current_namespace)
            .map(|c| c.project.clone()),
        // Commands are stateless definitions; no cross-project tracking.
        ResourceType::Command => None,
    }
}

/// Format a cross-project suggestion.
/// E.g.: "\n\n  did you mean: oj worker stop fix --project oddjobs?"
pub(super) fn format_cross_project_suggestion(
    command_prefix: &str,
    name: &str,
    project: &str,
) -> String {
    format!("\n\n  did you mean: {} {} --project {}?", command_prefix, name, project)
}

/// Generate a "did you mean" suggestion for a resource name.
///
/// Three-phase lookup:
/// 1. Fuzzy-match against names from runbooks on disk.
/// 2. Fuzzy-match against names from daemon state (current project).
/// 3. Cross-project exact match (suggests `--project`).
///
/// The two closures let callers specify how to collect candidate names:
/// - `collect_runbook_names()` → names from `.oj/runbooks/` (caller captures project root)
/// - `collect_state_names(state)` → names from materialized state
pub(super) fn suggest_for_resource(
    name: &str,
    project: &str,
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

    // 3. Check for wrong project (cross-project)
    let state = state.lock();
    if let Some(other_ns) = find_in_other_namespaces(resource_type, name, project, &state) {
        return format_cross_project_suggestion(command_prefix, name, &other_ns);
    }

    String::new()
}

/// Generate a "did you mean" suggestion from pre-collected candidates.
///
/// Use this variant when the state lock is already held (e.g., in query handlers).
pub(super) fn suggest_from_candidates(
    name: &str,
    project: &str,
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

    if let Some(other_ns) = find_in_other_namespaces(resource_type, name, project, state) {
        return format_cross_project_suggestion(command_prefix, name, &other_ns);
    }

    String::new()
}

#[cfg(test)]
#[path = "suggest_tests.rs"]
mod tests;
