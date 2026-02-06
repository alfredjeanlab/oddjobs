// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Status overview query handler.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use oj_core::{split_scoped_name, OwnerId, StepOutcome};
use oj_engine::breadcrumb::Breadcrumb;
use oj_storage::{MaterializedState, QueueItemStatus};

use crate::protocol::{
    AgentStatusEntry, JobStatusEntry, NamespaceStatus, QueueStatus, Response, WorkerSummary,
};

pub(super) fn handle_status_overview(
    state: &MaterializedState,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    start_time: Instant,
) -> Response {
    let uptime_secs = start_time.elapsed().as_secs();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Collect all namespaces seen across entities
    let mut ns_active: BTreeMap<String, Vec<JobStatusEntry>> = BTreeMap::new();
    let mut ns_escalated: BTreeMap<String, Vec<JobStatusEntry>> = BTreeMap::new();
    let mut ns_agents: BTreeMap<String, Vec<AgentStatusEntry>> = BTreeMap::new();

    for p in state.jobs.values() {
        if p.is_terminal() {
            continue;
        }

        let created_at_ms = p.step_history.first().map(|r| r.started_at_ms).unwrap_or(0);
        let elapsed_ms = now_ms.saturating_sub(created_at_ms);
        let last_activity_ms = p
            .step_history
            .last()
            .map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms))
            .unwrap_or(0);

        let waiting_reason = match p.step_history.last().map(|r| &r.outcome) {
            Some(StepOutcome::Waiting(reason)) => Some(reason.clone()),
            _ => None,
        };

        let escalate_source = match &p.step_status {
            oj_core::StepStatus::Waiting(Some(decision_id)) => state
                .decisions
                .get(decision_id.as_str())
                .map(|d| format!("{:?}", d.source).to_lowercase()),
            _ => None,
        };

        let entry = JobStatusEntry {
            id: p.id.clone(),
            name: p.name.clone(),
            kind: p.kind.clone(),
            step: p.step.clone(),
            step_status: p.step_status.to_string(),
            elapsed_ms,
            last_activity_ms,
            waiting_reason,
            escalate_source,
        };

        let ns = p.namespace.clone();
        if p.step_status.is_waiting() {
            ns_escalated.entry(ns).or_default().push(entry);
        } else {
            ns_active.entry(ns).or_default().push(entry);
        }
    }

    // Collect standalone agents from unified agents map
    let mut tracked_standalone_ids: HashSet<String> = HashSet::new();
    for record in state.agents.values() {
        // Only show standalone agents (job agents are shown via their job entry)
        let arid = match &record.owner {
            OwnerId::AgentRun(id) => id,
            OwnerId::Job(_) => continue,
        };

        // Skip terminal agents
        if matches!(
            record.status,
            oj_core::AgentRecordStatus::Exited | oj_core::AgentRecordStatus::Gone
        ) {
            continue;
        }

        // Derive command_name from the parent AgentRun record
        let command_name = state
            .agent_runs
            .get(arid.as_str())
            .map(|ar| ar.command_name.clone())
            .unwrap_or_default();

        tracked_standalone_ids.insert(record.agent_id.clone());
        ns_agents
            .entry(record.namespace.clone())
            .or_default()
            .push(AgentStatusEntry {
                agent_id: record.agent_id.clone(),
                agent_name: record.agent_name.clone(),
                command_name,
                status: format!("{}", record.status),
            });
    }

    // Fallback: agent_runs not yet in agents map (old WAL entries)
    for ar in state.agent_runs.values() {
        if ar.status.is_terminal() {
            continue;
        }
        let agent_id = ar.agent_id.clone().unwrap_or_else(|| ar.id.clone());
        if tracked_standalone_ids.contains(&agent_id) {
            continue;
        }
        ns_agents
            .entry(ar.namespace.clone())
            .or_default()
            .push(AgentStatusEntry {
                agent_id,
                agent_name: ar.agent_name.clone(),
                command_name: ar.command_name.clone(),
                status: ar.status.to_string(),
            });
    }

    // Collect workers grouped by namespace
    let mut ns_workers: BTreeMap<String, Vec<WorkerSummary>> = BTreeMap::new();
    for w in state.workers.values() {
        ns_workers
            .entry(w.namespace.clone())
            .or_default()
            .push(WorkerSummary {
                name: w.name.clone(),
                namespace: w.namespace.clone(),
                queue: w.queue_name.clone(),
                status: w.status.clone(),
                active: w.active_job_ids.len(),
                concurrency: w.concurrency,
                updated_at_ms: w
                    .active_job_ids
                    .iter()
                    .filter_map(|pid| state.jobs.get(pid))
                    .filter_map(|p| {
                        p.step_history
                            .last()
                            .map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms))
                    })
                    .max()
                    .unwrap_or(0),
            });
    }

    // Collect queue stats grouped by namespace
    let mut ns_queues: BTreeMap<String, Vec<QueueStatus>> = BTreeMap::new();
    for (scoped_key, items) in &state.queue_items {
        let (ns, queue_name) = split_scoped_name(scoped_key);

        let mut pending = 0;
        let mut active = 0;
        let mut dead = 0;
        for item in items {
            match item.status {
                QueueItemStatus::Pending => pending += 1,
                QueueItemStatus::Active => active += 1,
                QueueItemStatus::Dead => dead += 1,
                QueueItemStatus::Failed => pending += 1, // failed items pending retry
                QueueItemStatus::Completed => {}
            }
        }

        ns_queues
            .entry(ns.to_string())
            .or_default()
            .push(QueueStatus {
                name: queue_name.to_string(),
                pending,
                active,
                dead,
            });
    }

    // Count pending decisions grouped by namespace
    let mut ns_pending_decisions: BTreeMap<String, usize> = BTreeMap::new();
    for d in state.decisions.values() {
        if !d.is_resolved() {
            *ns_pending_decisions.entry(d.namespace.clone()).or_insert(0) += 1;
        }
    }

    // Collect orphaned jobs grouped by namespace
    let mut ns_orphaned = super::query_orphans::collect_orphan_status_entries(orphans, now_ms);

    // Build combined namespace set
    let mut all_namespaces: HashSet<String> = HashSet::new();
    for ns in ns_active.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_escalated.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_orphaned.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_workers.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_queues.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_agents.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_pending_decisions.keys() {
        all_namespaces.insert(ns.clone());
    }

    let mut namespaces: Vec<NamespaceStatus> = all_namespaces
        .into_iter()
        .map(|ns| NamespaceStatus {
            active_jobs: ns_active.remove(&ns).unwrap_or_default(),
            escalated_jobs: ns_escalated.remove(&ns).unwrap_or_default(),
            orphaned_jobs: ns_orphaned.remove(&ns).unwrap_or_default(),
            workers: ns_workers.remove(&ns).unwrap_or_default(),
            queues: ns_queues.remove(&ns).unwrap_or_default(),
            active_agents: ns_agents.remove(&ns).unwrap_or_default(),
            pending_decisions: ns_pending_decisions.remove(&ns).unwrap_or_default(),
            namespace: ns,
        })
        .collect();
    namespaces.sort_by(|a, b| a.namespace.cmp(&b.namespace));

    Response::StatusOverview {
        uptime_secs,
        namespaces,
    }
}
