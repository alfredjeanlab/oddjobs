// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared helper functions for state event handlers.

use oj_core::{AgentRecord, AgentRecordStatus, Decision, Job, OwnerId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::QueueItem;

/// Current epoch time in milliseconds.
pub(crate) fn epoch_ms_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Get a value by exact ID or unique prefix.
///
/// Matches against both the full key and the suffix after the type prefix
/// (e.g. "job-", "agt-"). This allows short IDs displayed without their
/// type prefix to resolve back to the full entry.
pub(crate) fn find_by_prefix<'a, V>(map: &'a HashMap<String, V>, id: &str) -> Option<&'a V> {
    if let Some(val) = map.get(id) {
        return Some(val);
    }
    let matches: Vec<_> = map.iter().filter(|(k, _)| oj_core::id::prefix_matches(k, id)).collect();
    if matches.len() == 1 {
        Some(matches[0].1)
    } else {
        None
    }
}

/// Find a queue item by ID within a queue's item list.
pub(crate) fn find_queue_item_mut<'a>(
    items: &'a mut [QueueItem],
    item_id: &str,
) -> Option<&'a mut QueueItem> {
    items.iter_mut().find(|i| i.id == item_id)
}

/// Apply a mutation to a job only if it hasn't reached a terminal state.
pub(crate) fn apply_if_not_terminal(
    jobs: &mut HashMap<String, Job>,
    job_id: &str,
    f: impl FnOnce(&mut Job),
) {
    if let Some(job) = jobs.get_mut(job_id) {
        if !job.is_terminal() {
            f(job);
        }
    }
}

/// Create a new [`AgentRecord`] with the current timestamp.
pub(crate) fn create_agent_record(
    agent_id: &str,
    agent_name: String,
    owner: OwnerId,
    project: String,
    workspace_path: PathBuf,
    status: AgentRecordStatus,
) -> AgentRecord {
    let now = epoch_ms_now();
    AgentRecord {
        agent_id: agent_id.to_string(),
        agent_name,
        owner,
        project,
        workspace_path,
        status,
        runtime: oj_core::AgentRuntime::default(),
        auth_token: None,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

/// Remove unresolved decisions owned by a specific owner.
pub(crate) fn cleanup_unresolved_decisions_for_owner(
    decisions: &mut HashMap<String, Decision>,
    owner: &OwnerId,
) {
    decisions.retain(|_, d| d.owner != *owner || d.is_resolved());
}
