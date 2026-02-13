// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Orphan query handlers.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{Breadcrumb, StepStatusKind};

use crate::protocol::{
    AgentSummary, JobDetail, JobStatusEntry, JobSummary, OrphanSummary, Response,
};

/// Handle ListOrphans query by converting breadcrumbs to OrphanSummary.
pub(super) fn handle_list_orphans(orphans: &Arc<Mutex<Vec<Breadcrumb>>>) -> Response {
    let orphans = orphans.lock();
    let summaries = orphans.iter().map(OrphanSummary::from).collect();
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
    let idx = orphans.iter().position(|bc| bc.job_id == id || bc.job_id.starts_with(id));

    match idx {
        Some(i) => {
            let removed = orphans.remove(i);
            // Clean up all associated files (breadcrumb, job log, agent files)
            let crumb = oj_core::log_paths::breadcrumb_path(logs_path, &removed.job_id);
            let _ = std::fs::remove_file(&crumb);
            let log = oj_core::log_paths::job_log_path(logs_path, &removed.job_id);
            let _ = std::fs::remove_file(&log);
            for agent in &removed.agents {
                let agent_log = oj_core::log_paths::agent_log_path(logs_path, &agent.agent_id);
                let _ = std::fs::remove_file(&agent_log);
                let agent_dir =
                    oj_core::log_paths::agent_session_log_dir(logs_path, &agent.agent_id);
                let _ = std::fs::remove_dir_all(&agent_dir);
            }
            Response::Ok
        }
        None => Response::Error { message: format!("orphan not found: {}", id) },
    }
}

/// Append orphaned jobs as `JobSummary` entries to a job list.
pub(super) fn append_orphan_summaries(
    jobs: &mut Vec<JobSummary>,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
) {
    let orphans = orphans.lock();
    for bc in orphans.iter() {
        let updated_at_ms = parse_rfc3339_to_epoch_ms(&bc.updated_at);
        jobs.push(JobSummary {
            id: oj_core::JobId::new(&bc.job_id),
            name: bc.name.clone(),
            kind: bc.kind.clone(),
            step: bc.current_step.clone(),
            step_status: StepStatusKind::Orphaned,
            created_at_ms: updated_at_ms,
            updated_at_ms,
            project: bc.project.clone(),
            retries: 0,
        });
    }
}

/// Look up an orphan by exact ID or prefix, returning a `JobDetail`.
pub(super) fn find_orphan_detail(
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    id: &str,
) -> Option<Box<JobDetail>> {
    let orphans = orphans.lock();
    orphans.iter().find(|bc| bc.job_id == id || bc.job_id.starts_with(id)).map(|bc| {
        Box::new(JobDetail {
            id: oj_core::JobId::new(&bc.job_id),
            name: bc.name.clone(),
            kind: bc.kind.clone(),
            step: bc.current_step.clone(),
            step_status: StepStatusKind::Orphaned,
            vars: bc.vars.clone(),
            workspace_path: bc.workspace_root.clone(),
            error: Some("Job was not recovered from WAL/snapshot".to_string()),
            steps: Vec::new(),
            agents: bc
                .agents
                .iter()
                .map(|a| AgentSummary {
                    owner: oj_core::JobId::new(&bc.job_id).into(),
                    step_name: bc.current_step.clone(),
                    agent_id: oj_core::AgentId::new(&a.agent_id),
                    agent_name: None,
                    project: bc.project.clone(),
                    status: "orphaned".to_string(),
                    files_read: 0,
                    files_written: 0,
                    commands_run: 0,
                    exit_reason: None,
                    updated_at_ms: 0,
                })
                .collect(),
            project: bc.project.clone(),
        })
    })
}

/// Resolve an orphan job ID by exact match or prefix, returning the full ID.
pub(super) fn find_orphan_id(orphans: &Arc<Mutex<Vec<Breadcrumb>>>, id: &str) -> Option<String> {
    let orphans = orphans.lock();
    orphans
        .iter()
        .find(|bc| bc.job_id == id || bc.job_id.starts_with(id))
        .map(|bc| bc.job_id.clone())
}

/// Collect orphaned jobs grouped by project for status overview.
pub(super) fn collect_orphan_status_entries(
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    now_ms: u64,
) -> BTreeMap<String, Vec<JobStatusEntry>> {
    let mut ns_orphaned: BTreeMap<String, Vec<JobStatusEntry>> = BTreeMap::new();
    let orphans = orphans.lock();
    for bc in orphans.iter() {
        let updated_at_ms = parse_rfc3339_to_epoch_ms(&bc.updated_at);
        let elapsed_ms = now_ms.saturating_sub(updated_at_ms);
        ns_orphaned.entry(bc.project.clone()).or_default().push(JobStatusEntry {
            id: oj_core::JobId::new(&bc.job_id),
            name: bc.name.clone(),
            kind: bc.kind.clone(),
            step: bc.current_step.clone(),
            step_status: StepStatusKind::Orphaned,
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
