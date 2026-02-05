// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Orphan query handlers.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_engine::breadcrumb::Breadcrumb;

use crate::protocol::{
    AgentSummary, OrphanAgent, OrphanSummary, PipelineDetail, PipelineStatusEntry, PipelineSummary,
    Response,
};

/// Handle ListOrphans query by converting breadcrumbs to OrphanSummary.
pub(super) fn handle_list_orphans(orphans: &Arc<Mutex<Vec<Breadcrumb>>>) -> Response {
    let orphans = orphans.lock();
    let summaries = orphans
        .iter()
        .map(|bc| OrphanSummary {
            pipeline_id: bc.pipeline_id.clone(),
            project: bc.project.clone(),
            kind: bc.kind.clone(),
            name: bc.name.clone(),
            current_step: bc.current_step.clone(),
            step_status: bc.step_status.clone(),
            workspace_root: bc.workspace_root.clone(),
            agents: bc
                .agents
                .iter()
                .map(|a| OrphanAgent {
                    agent_id: a.agent_id.clone(),
                    session_name: a.session_name.clone(),
                    log_path: a.log_path.clone(),
                })
                .collect(),
            updated_at: bc.updated_at.clone(),
        })
        .collect();
    Response::Orphans { orphans: summaries }
}

/// Handle DismissOrphan query by removing the orphan from the registry and cleaning up its files.
pub(super) fn handle_dismiss_orphan(
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    id: &str,
    logs_path: &Path,
) -> Response {
    let mut orphans = orphans.lock();

    // Find by exact match or prefix
    let idx = orphans
        .iter()
        .position(|bc| bc.pipeline_id == id || bc.pipeline_id.starts_with(id));

    match idx {
        Some(i) => {
            let removed = orphans.remove(i);
            // Clean up all associated files (breadcrumb, pipeline log, agent files)
            let crumb = oj_engine::log_paths::breadcrumb_path(logs_path, &removed.pipeline_id);
            let _ = std::fs::remove_file(&crumb);
            let log = oj_engine::log_paths::pipeline_log_path(logs_path, &removed.pipeline_id);
            let _ = std::fs::remove_file(&log);
            for agent in &removed.agents {
                let agent_log = oj_engine::log_paths::agent_log_path(logs_path, &agent.agent_id);
                let _ = std::fs::remove_file(&agent_log);
                let agent_dir =
                    oj_engine::log_paths::agent_session_log_dir(logs_path, &agent.agent_id);
                let _ = std::fs::remove_dir_all(&agent_dir);
            }
            Response::Ok
        }
        None => Response::Error {
            message: format!("orphan not found: {}", id),
        },
    }
}

/// Append orphaned pipelines as `PipelineSummary` entries to a pipeline list.
pub(super) fn append_orphan_summaries(
    pipelines: &mut Vec<PipelineSummary>,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
) {
    let orphans = orphans.lock();
    for bc in orphans.iter() {
        let updated_at_ms = parse_rfc3339_to_epoch_ms(&bc.updated_at);
        pipelines.push(PipelineSummary {
            id: bc.pipeline_id.clone(),
            name: bc.name.clone(),
            kind: bc.kind.clone(),
            step: bc.current_step.clone(),
            step_status: "orphaned".to_string(),
            created_at_ms: updated_at_ms,
            updated_at_ms,
            namespace: bc.project.clone(),
            retry_count: 0,
        });
    }
}

/// Look up an orphan by exact ID or prefix, returning a `PipelineDetail`.
pub(super) fn find_orphan_detail(
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    id: &str,
) -> Option<Box<PipelineDetail>> {
    let orphans = orphans.lock();
    orphans
        .iter()
        .find(|bc| bc.pipeline_id == id || bc.pipeline_id.starts_with(id))
        .map(|bc| {
            Box::new(PipelineDetail {
                id: bc.pipeline_id.clone(),
                name: bc.name.clone(),
                kind: bc.kind.clone(),
                step: bc.current_step.clone(),
                step_status: "orphaned".to_string(),
                vars: bc.vars.clone(),
                workspace_path: bc.workspace_root.clone(),
                session_id: bc.agents.iter().find_map(|a| a.session_name.clone()),
                error: Some("Pipeline was not recovered from WAL/snapshot".to_string()),
                steps: Vec::new(),
                agents: bc
                    .agents
                    .iter()
                    .map(|a| AgentSummary {
                        pipeline_id: bc.pipeline_id.clone(),
                        step_name: bc.current_step.clone(),
                        agent_id: a.agent_id.clone(),
                        agent_name: None,
                        namespace: Some(bc.project.clone()),
                        status: "orphaned".to_string(),
                        files_read: 0,
                        files_written: 0,
                        commands_run: 0,
                        exit_reason: None,
                        updated_at_ms: 0,
                    })
                    .collect(),
                namespace: bc.project.clone(),
            })
        })
}

/// Resolve an orphan pipeline ID by exact match or prefix, returning the full ID.
pub(super) fn find_orphan_id(orphans: &Arc<Mutex<Vec<Breadcrumb>>>, id: &str) -> Option<String> {
    let orphans = orphans.lock();
    orphans
        .iter()
        .find(|bc| bc.pipeline_id == id || bc.pipeline_id.starts_with(id))
        .map(|bc| bc.pipeline_id.clone())
}

/// Collect orphaned pipelines grouped by namespace for status overview.
pub(super) fn collect_orphan_status_entries(
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    now_ms: u64,
) -> BTreeMap<String, Vec<PipelineStatusEntry>> {
    let mut ns_orphaned: BTreeMap<String, Vec<PipelineStatusEntry>> = BTreeMap::new();
    let orphans = orphans.lock();
    for bc in orphans.iter() {
        let updated_at_ms = parse_rfc3339_to_epoch_ms(&bc.updated_at);
        let elapsed_ms = now_ms.saturating_sub(updated_at_ms);
        ns_orphaned
            .entry(bc.project.clone())
            .or_default()
            .push(PipelineStatusEntry {
                id: bc.pipeline_id.clone(),
                name: bc.name.clone(),
                kind: bc.kind.clone(),
                step: bc.current_step.clone(),
                step_status: "orphaned".to_string(),
                elapsed_ms,
                last_activity_ms: updated_at_ms,
                waiting_reason: None,
                escalate_source: None,
            });
    }
    ns_orphaned
}

/// Parse an RFC 3339 UTC timestamp (e.g. "2026-01-15T10:30:00Z") to epoch milliseconds.
/// Returns 0 on parse failure.
fn parse_rfc3339_to_epoch_ms(s: &str) -> u64 {
    // Expected format: YYYY-MM-DDTHH:MM:SSZ
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return 0;
    }
    let year: u64 = s[0..4].parse().unwrap_or(0);
    let month: u64 = s[5..7].parse().unwrap_or(0);
    let day: u64 = s[8..10].parse().unwrap_or(0);
    let hour: u64 = s[11..13].parse().unwrap_or(0);
    let min: u64 = s[14..16].parse().unwrap_or(0);
    let sec: u64 = s[17..19].parse().unwrap_or(0);

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return 0;
    }

    // Days from year 1970 to start of `year`
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let m = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    (days * 86400 + hour * 3600 + min * 60 + sec) * 1000
}
