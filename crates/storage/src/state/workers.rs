// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker and cron event handlers.

use oj_core::{scoped_name, Event};

use super::types::{CronRecord, WorkerRecord};
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::WorkerStarted {
            worker_name,
            project_root,
            runbook_hash,
            queue_name,
            concurrency,
            namespace,
        } => {
            let key = scoped_name(namespace, worker_name);
            // Preserve active_job_ids from before restart
            let existing_job_ids = state
                .workers
                .get(&key)
                .map(|w| w.active_job_ids.clone())
                .unwrap_or_default();

            if !namespace.is_empty() {
                state
                    .project_roots
                    .insert(namespace.clone(), project_root.clone());
            }
            state.workers.insert(
                key,
                WorkerRecord {
                    name: worker_name.clone(),
                    namespace: namespace.clone(),
                    project_root: project_root.clone(),
                    runbook_hash: runbook_hash.clone(),
                    status: "running".to_string(),
                    active_job_ids: existing_job_ids,
                    queue_name: queue_name.clone(),
                    concurrency: *concurrency,
                },
            );
        }

        Event::WorkerItemDispatched {
            worker_name,
            job_id,
            namespace,
            ..
        } => {
            let key = scoped_name(namespace, worker_name);
            if let Some(record) = state.workers.get_mut(&key) {
                let pid = job_id.to_string();
                if !record.active_job_ids.contains(&pid) {
                    record.active_job_ids.push(pid);
                }
            }
        }

        Event::WorkerStopped {
            worker_name,
            namespace,
        } => {
            let key = scoped_name(namespace, worker_name);
            if let Some(record) = state.workers.get_mut(&key) {
                record.status = "stopped".to_string();
            }
        }

        Event::WorkerResized {
            worker_name,
            concurrency,
            namespace,
        } => {
            let key = scoped_name(namespace, worker_name);
            if let Some(record) = state.workers.get_mut(&key) {
                record.concurrency = *concurrency;
            }
        }

        Event::WorkerDeleted {
            worker_name,
            namespace,
        } => {
            let key = scoped_name(namespace, worker_name);
            state.workers.remove(&key);
        }

        Event::CronStarted {
            cron_name,
            project_root,
            runbook_hash,
            interval,
            run_target,
            namespace,
        } => {
            if !namespace.is_empty() {
                state
                    .project_roots
                    .insert(namespace.clone(), project_root.clone());
            }
            let key = scoped_name(namespace, cron_name);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            // Preserve last_fired_at_ms across restarts (re-emitted CronStarted)
            let last_fired_at_ms = state.crons.get(&key).and_then(|r| r.last_fired_at_ms);
            state.crons.insert(
                key,
                CronRecord {
                    name: cron_name.clone(),
                    namespace: namespace.clone(),
                    project_root: project_root.clone(),
                    runbook_hash: runbook_hash.clone(),
                    status: "running".to_string(),
                    interval: interval.clone(),
                    run_target: run_target.clone(),
                    started_at_ms: now_ms,
                    last_fired_at_ms,
                },
            );
        }

        Event::CronStopped {
            cron_name,
            namespace,
        } => {
            let key = scoped_name(namespace, cron_name);
            if let Some(record) = state.crons.get_mut(&key) {
                record.status = "stopped".to_string();
            }
        }

        Event::CronFired {
            cron_name,
            namespace,
            ..
        } => {
            let key = scoped_name(namespace, cron_name);
            if let Some(record) = state.crons.get_mut(&key) {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                record.last_fired_at_ms = Some(now_ms);
            }
        }

        Event::CronDeleted {
            cron_name,
            namespace,
        } => {
            let key = scoped_name(namespace, cron_name);
            state.crons.remove(&key);
        }

        _ => {}
    }
}
