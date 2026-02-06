// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Log retrieval query handlers.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::scoped_name;
use oj_engine::breadcrumb::Breadcrumb;
use oj_storage::MaterializedState;

use crate::protocol::Response;

pub(super) fn handle_get_agent_logs(
    id: String,
    step: Option<String>,
    lines: usize,
    state: &MaterializedState,
    logs_path: &Path,
) -> Response {
    use oj_engine::log_paths::agent_log_path;

    // Look up job to find agent_ids from step history
    let job = state.get_job(&id);

    let (content, steps, log_path) = if let Some(step_name) = step {
        // Single step: find agent_id for that step
        let agent_id = job.and_then(|p| {
            p.step_history
                .iter()
                .find(|r| r.name == step_name)
                .and_then(|r| r.agent_id.clone())
        });

        if let Some(aid) = agent_id {
            let path = agent_log_path(logs_path, &aid);
            let text = read_log_file(&path, lines);
            (text, vec![step_name], path)
        } else {
            (String::new(), vec![step_name], logs_path.join("agent"))
        }
    } else {
        // All steps: collect agent logs from step history
        let mut content = String::new();
        let mut step_names = Vec::new();
        let mut last_path = logs_path.join("agent");

        if let Some(p) = job {
            for record in &p.step_history {
                if let Some(ref aid) = record.agent_id {
                    step_names.push(record.name.clone());
                    let path = agent_log_path(logs_path, aid);
                    last_path = path.clone();

                    if let Ok(text) = std::fs::read_to_string(&path) {
                        if !content.is_empty() {
                            content.push('\n');
                        }
                        content.push_str(&format!("=== {} ===\n", record.name));
                        if lines > 0 {
                            let all_lines: Vec<&str> = text.lines().collect();
                            let start = all_lines.len().saturating_sub(lines);
                            content.push_str(&all_lines[start..].join("\n"));
                        } else {
                            content.push_str(&text);
                        }
                    }
                }
            }
        }

        (content, step_names, last_path)
    };

    Response::AgentLogs {
        log_path,
        content,
        steps,
    }
}

pub(super) fn handle_get_job_logs(
    id: String,
    lines: usize,
    state: &MaterializedState,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    logs_path: &Path,
) -> Response {
    use oj_engine::log_paths::job_log_path;

    // Resolve job ID (supports prefix matching), falling back to orphans
    let full_id = state
        .get_job(&id)
        .map(|p| p.id.clone())
        .or_else(|| super::query_orphans::find_orphan_id(orphans, &id))
        .unwrap_or(id);

    let log_path = job_log_path(logs_path, &full_id);
    let content = match std::fs::read_to_string(&log_path) {
        Ok(text) => {
            if lines > 0 {
                let all_lines: Vec<&str> = text.lines().collect();
                let start = all_lines.len().saturating_sub(lines);
                all_lines[start..].join("\n")
            } else {
                text
            }
        }
        Err(_) => String::new(),
    };
    Response::JobLogs { log_path, content }
}

pub(super) fn handle_get_worker_logs(
    name: String,
    namespace: String,
    lines: usize,
    project_root: Option<std::path::PathBuf>,
    state: &MaterializedState,
    logs_path: &Path,
) -> Response {
    use oj_engine::log_paths::worker_log_path;

    let scoped = scoped_name(&namespace, &name);
    let log_path = worker_log_path(logs_path, &scoped);

    // If log exists, return it (worker was active at some point)
    if log_path.exists() {
        let content = read_log_file(&log_path, lines);
        return Response::WorkerLogs { log_path, content };
    }

    // Log doesn't exist — check if worker is known
    let in_state = state.workers.contains_key(&scoped);
    let in_runbook = project_root.as_ref().is_some_and(|root| {
        oj_runbook::find_runbook_by_worker(&root.join(".oj/runbooks"), &name)
            .ok()
            .flatten()
            .is_some()
    });

    if in_state || in_runbook {
        // Worker exists but no logs yet
        Response::WorkerLogs {
            log_path,
            content: String::new(),
        }
    } else {
        // Worker not found — suggest
        use super::super::suggest;
        let mut candidates: Vec<String> = state
            .workers
            .values()
            .filter(|w| w.namespace == namespace)
            .map(|w| w.name.clone())
            .collect();
        if let Some(ref root) = project_root {
            let runbook_workers =
                oj_runbook::collect_all_workers(&root.join(".oj/runbooks")).unwrap_or_default();
            for (wname, _) in runbook_workers {
                if !candidates.contains(&wname) {
                    candidates.push(wname);
                }
            }
        }

        let hint = suggest::suggest_from_candidates(
            &name,
            &namespace,
            "oj worker logs",
            state,
            suggest::ResourceType::Worker,
            &candidates,
        );

        if hint.is_empty() {
            Response::WorkerLogs {
                log_path,
                content: String::new(),
            }
        } else {
            Response::Error {
                message: format!("unknown worker: {}{}", name, hint),
            }
        }
    }
}

pub(super) fn handle_get_cron_logs(
    name: String,
    namespace: String,
    lines: usize,
    project_root: Option<std::path::PathBuf>,
    state: &MaterializedState,
    logs_path: &Path,
) -> Response {
    use oj_engine::log_paths::cron_log_path;

    let scoped = scoped_name(&namespace, &name);
    let log_path = cron_log_path(logs_path, &scoped);

    // If log exists, return it
    if log_path.exists() {
        let content = read_log_file(&log_path, lines);
        return Response::CronLogs { log_path, content };
    }

    // Log doesn't exist — check if cron is known
    let in_state = state.crons.values().any(|c| c.name == name);
    let in_runbook = project_root.as_ref().is_some_and(|root| {
        oj_runbook::find_runbook_by_cron(&root.join(".oj/runbooks"), &name)
            .ok()
            .flatten()
            .is_some()
    });

    if in_state || in_runbook {
        Response::CronLogs {
            log_path,
            content: String::new(),
        }
    } else {
        // Cron not found — suggest
        use super::super::suggest;
        let mut candidates: Vec<String> = state.crons.values().map(|c| c.name.clone()).collect();
        if let Some(ref root) = project_root {
            let runbook_crons =
                oj_runbook::collect_all_crons(&root.join(".oj/runbooks")).unwrap_or_default();
            for (cname, _) in runbook_crons {
                if !candidates.contains(&cname) {
                    candidates.push(cname);
                }
            }
        }

        let hint = suggest::suggest_from_candidates(
            &name,
            "",
            "oj cron logs",
            state,
            suggest::ResourceType::Cron,
            &candidates,
        );

        if hint.is_empty() {
            Response::CronLogs {
                log_path,
                content: String::new(),
            }
        } else {
            Response::Error {
                message: format!("unknown cron: {}{}", name, hint),
            }
        }
    }
}

pub(super) fn handle_get_queue_logs(
    queue_name: String,
    namespace: String,
    lines: usize,
    logs_path: &Path,
) -> Response {
    use oj_engine::log_paths::queue_log_path;

    let scoped = scoped_name(&namespace, &queue_name);
    let path = queue_log_path(logs_path, &scoped);
    let content = read_log_file(&path, lines);
    Response::QueueLogs {
        log_path: path,
        content,
    }
}

/// Read a log file, returning the last N lines (or all if lines == 0).
fn read_log_file(path: &Path, lines: usize) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            if lines > 0 {
                let all_lines: Vec<&str> = text.lines().collect();
                let start = all_lines.len().saturating_sub(lines);
                all_lines[start..].join("\n")
            } else {
                text
            }
        }
        Err(_) => String::new(),
    }
}
