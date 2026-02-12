// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker and cron event handlers.

use oj_core::{scoped_name, Event};

use super::types::{CronRecord, WorkerRecord};
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::WorkerStarted {
            worker,
            project_path,
            runbook_hash,
            queue,
            concurrency,
            project,
        } => {
            let key = scoped_name(project, worker);
            // Preserve active_job_ids and item_owners from before restart
            let (existing_job_ids, existing_item_owners) = state
                .workers
                .get(&key)
                .map(|w| (w.active.clone(), w.owners.clone()))
                .unwrap_or_default();

            if !project.is_empty() {
                state.project_paths.insert(project.clone(), project_path.clone());
            }
            state.workers.insert(
                key,
                WorkerRecord {
                    name: worker.clone(),
                    project: project.clone(),
                    project_path: project_path.clone(),
                    runbook_hash: runbook_hash.clone(),
                    status: "running".to_string(),
                    active: existing_job_ids,
                    queue: queue.clone(),
                    concurrency: *concurrency,
                    owners: existing_item_owners,
                },
            );
        }

        Event::WorkerDispatched { worker, item_id, owner, project } => {
            let key = scoped_name(project, worker);
            if let Some(record) = state.workers.get_mut(&key) {
                let pid = owner.to_string();
                if !record.active.contains(&pid) {
                    record.active.push(pid.clone());
                }
                record.owners.insert(pid, item_id.clone());
            }
        }

        Event::WorkerStopped { worker, project } => {
            let key = scoped_name(project, worker);
            if let Some(record) = state.workers.get_mut(&key) {
                record.status = "stopped".to_string();
            }
        }

        Event::WorkerResized { worker, concurrency, project } => {
            let key = scoped_name(project, worker);
            if let Some(record) = state.workers.get_mut(&key) {
                record.concurrency = *concurrency;
            }
        }

        Event::WorkerDeleted { worker, project } => {
            let key = scoped_name(project, worker);
            state.workers.remove(&key);
        }

        Event::CronStarted { cron, project, project_path, runbook_hash, interval, target } => {
            if !project.is_empty() {
                state.project_paths.insert(project.clone(), project_path.clone());
            }
            let key = scoped_name(project, cron);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            // Preserve last_fired_at_ms across restarts (re-emitted CronStarted)
            let last_fired_at_ms = state.crons.get(&key).and_then(|r| r.last_fired_at_ms);
            state.crons.insert(
                key,
                CronRecord {
                    name: cron.clone(),
                    project: project.clone(),
                    project_path: project_path.clone(),
                    runbook_hash: runbook_hash.clone(),
                    status: "running".to_string(),
                    interval: interval.clone(),
                    target: target.clone(),
                    started_at_ms: now_ms,
                    last_fired_at_ms,
                },
            );
        }

        Event::CronStopped { cron, project } => {
            let key = scoped_name(project, cron);
            if let Some(record) = state.crons.get_mut(&key) {
                record.status = "stopped".to_string();
            }
        }

        Event::CronFired { cron, project, .. } => {
            let key = scoped_name(project, cron);
            if let Some(record) = state.crons.get_mut(&key) {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                record.last_fired_at_ms = Some(now_ms);
            }
        }

        Event::CronDeleted { cron, project } => {
            let key = scoped_name(project, cron);
            state.crons.remove(&key);
        }

        _ => {}
    }
}
