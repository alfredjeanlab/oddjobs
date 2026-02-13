// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Status overview query handler.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::storage::{MaterializedState, QueueItemStatus};
use oj_core::{split_scoped_name, OwnerId, StepOutcome};
use oj_core::{Breadcrumb, MetricsHealth};

use crate::protocol::{
    AgentStatusEntry, CronSummary, JobStatusEntry, MetricsHealthSummary, ProjectStatus,
    QueueStatus, Response, WorkerSummary,
};

pub(super) fn handle_status_overview(
    state: &MaterializedState,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    metrics_health: &Arc<Mutex<MetricsHealth>>,
    start_time: Instant,
) -> Response {
    let uptime_secs = start_time.elapsed().as_secs();
    let now_ms =
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;

    // Collect all namespaces seen across entities
    let mut ns_active: BTreeMap<String, Vec<JobStatusEntry>> = BTreeMap::new();
    let mut ns_escalated: BTreeMap<String, Vec<JobStatusEntry>> = BTreeMap::new();
    let mut ns_suspended: BTreeMap<String, Vec<JobStatusEntry>> = BTreeMap::new();
    let mut ns_agents: BTreeMap<String, Vec<AgentStatusEntry>> = BTreeMap::new();

    for p in state.jobs.values() {
        if p.is_suspended() {
            let entry = JobStatusEntry::from_job(p, now_ms, None, None);
            ns_suspended.entry(p.project.clone()).or_default().push(entry);
            continue;
        }
        if p.is_terminal() {
            continue;
        }

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

        let entry = JobStatusEntry::from_job(p, now_ms, waiting_reason, escalate_source);

        let ns = p.project.clone();
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
            OwnerId::Crew(id) => id,
            OwnerId::Job(_) => continue,
        };

        // Skip terminal agents
        if matches!(
            record.status,
            oj_core::AgentRecordStatus::Exited | oj_core::AgentRecordStatus::Gone
        ) {
            continue;
        }

        // Derive command_name from the parent Crew record
        let command_name =
            state.crew.get(arid.as_str()).map(|run| run.command_name.clone()).unwrap_or_default();

        tracked_standalone_ids.insert(record.agent_id.clone());
        ns_agents.entry(record.project.clone()).or_default().push(AgentStatusEntry {
            agent_id: oj_core::AgentId::new(&record.agent_id),
            agent_name: record.agent_name.clone(),
            command_name,
            status: format!("{}", record.status),
        });
    }

    // Fallback: crew not yet in agents map (old WAL entries)
    for run in state.crew.values() {
        if run.status.is_terminal() {
            continue;
        }
        let agent_id = run.agent_id.clone().unwrap_or_else(|| run.id.clone());
        if tracked_standalone_ids.contains(&agent_id) {
            continue;
        }
        ns_agents.entry(run.project.clone()).or_default().push(AgentStatusEntry {
            agent_id: oj_core::AgentId::new(&agent_id),
            agent_name: run.agent_name.clone(),
            command_name: run.command_name.clone(),
            status: run.status.to_string(),
        });
    }

    // Collect workers grouped by project
    let mut ns_workers: BTreeMap<String, Vec<WorkerSummary>> = BTreeMap::new();
    for w in state.workers.values() {
        let updated_at_ms = super::worker_updated_at_ms(w, state);
        ns_workers
            .entry(w.project.clone())
            .or_default()
            .push(WorkerSummary::from_worker(w, updated_at_ms));
    }

    // Collect crons grouped by project
    let mut ns_crons: BTreeMap<String, Vec<CronSummary>> = BTreeMap::new();
    for c in state.crons.values() {
        let time = super::query_crons::cron_time_display(c, now_ms);
        ns_crons.entry(c.project.clone()).or_default().push(CronSummary::from_cron(c, time));
    }

    // Collect queue stats grouped by project
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

        ns_queues.entry(ns.to_string()).or_default().push(QueueStatus {
            name: queue_name.to_string(),
            pending,
            active,
            dead,
        });
    }

    // Count pending decisions grouped by project
    let mut ns_pending_decisions: BTreeMap<String, usize> = BTreeMap::new();
    for d in state.decisions.values() {
        if !d.is_resolved() {
            *ns_pending_decisions.entry(d.project.clone()).or_insert(0) += 1;
        }
    }

    // Collect orphaned jobs grouped by project
    let mut ns_orphaned = super::query_orphans::collect_orphan_status_entries(orphans, now_ms);

    // Build combined project set
    let mut all_namespaces: HashSet<String> = HashSet::new();
    for ns in ns_active.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_escalated.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_suspended.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_orphaned.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_workers.keys() {
        all_namespaces.insert(ns.clone());
    }
    for ns in ns_crons.keys() {
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

    let mut namespaces: Vec<ProjectStatus> = all_namespaces
        .into_iter()
        .map(|ns| ProjectStatus {
            active_jobs: ns_active.remove(&ns).unwrap_or_default(),
            escalated_jobs: ns_escalated.remove(&ns).unwrap_or_default(),
            suspended_jobs: ns_suspended.remove(&ns).unwrap_or_default(),
            orphaned_jobs: ns_orphaned.remove(&ns).unwrap_or_default(),
            workers: ns_workers.remove(&ns).unwrap_or_default(),
            crons: ns_crons.remove(&ns).unwrap_or_default(),
            queues: ns_queues.remove(&ns).unwrap_or_default(),
            active_agents: ns_agents.remove(&ns).unwrap_or_default(),
            pending_decisions: ns_pending_decisions.remove(&ns).unwrap_or_default(),
            project: ns,
        })
        .collect();
    namespaces.sort_by(|a, b| a.project.cmp(&b.project));

    let metrics = {
        let mh = metrics_health.lock();
        MetricsHealthSummary::from(&*mh)
    };

    Response::StatusOverview { uptime_secs, projects: namespaces, metrics_health: Some(metrics) }
}
