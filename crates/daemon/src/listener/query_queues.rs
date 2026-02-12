// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue query helpers.

use std::path::Path;

use oj_core::{scoped_name, split_scoped_name};
use oj_storage::MaterializedState;

use crate::protocol::{QueueSummary, Response};

/// Build a `Response::Queues` listing all known queues across all namespaces,
/// plus any empty queues defined in the local runbook.
pub(super) fn list_queues(
    state: &MaterializedState,
    project_path: &Path,
    project: &str,
) -> Response {
    let mut queues: Vec<QueueSummary> = state
        .queue_items
        .iter()
        .map(|(scoped_key, items)| {
            let (project, queue) = split_scoped_name(scoped_key);

            let workers: Vec<String> = state
                .workers
                .values()
                .filter(|w| w.queue == queue && w.project == project)
                .map(|w| w.name.clone())
                .collect();

            let poll_meta = state.poll_meta.get(scoped_key);

            QueueSummary {
                name: queue.to_string(),
                project: project.to_string(),
                queue_type: "persisted".to_string(),
                item_count: items.len(),
                workers,
                last_poll_count: poll_meta.map(|m| m.last_item_count),
                last_polled_at_ms: poll_meta.map(|m| m.last_polled_at_ms),
            }
        })
        .collect();

    // Also include queues defined in the local runbook that may have no items yet
    let runbook_dir = project_path.join(".oj/runbooks");
    let queue_defs = oj_runbook::collect_all_queues(&runbook_dir).unwrap_or_default();
    for (name, def) in queue_defs {
        let already_listed = queues.iter().any(|q| q.name == name && q.project == project);
        if !already_listed {
            let queue_type = match def.queue_type {
                oj_runbook::QueueType::External => "external",
                oj_runbook::QueueType::Persisted => "persisted",
            };
            let key = scoped_name(project, &name);
            let poll_meta = state.poll_meta.get(&key);
            queues.push(QueueSummary {
                name,
                project: project.to_string(),
                queue_type: queue_type.to_string(),
                item_count: 0,
                workers: vec![],
                last_poll_count: poll_meta.map(|m| m.last_item_count),
                last_polled_at_ms: poll_meta.map(|m| m.last_polled_at_ms),
            });
        }
    }

    queues.sort_by(|a, b| (&a.project, &a.name).cmp(&(&b.project, &b.name)));
    Response::Queues { queues }
}
