// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Log retrieval query handlers.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::scoped_name;
use oj_engine::breadcrumb::Breadcrumb;
use oj_engine::log_paths::{
    agent_log_path, cron_log_path, job_log_path, queue_log_path, worker_log_path,
};
use oj_storage::MaterializedState;

use super::super::suggest;
use crate::protocol::Response;

pub(super) fn handle_get_agent_logs(
    id: String,
    step: Option<String>,
    lines: usize,
    offset: u64,
    state: &MaterializedState,
    logs_path: &Path,
) -> Response {
    // Look up job to find agent_ids from step history
    let job = state.get_job(&id);

    // If no job found, try resolving `id` as an agent ID (prefix match)
    if job.is_none() {
        // Check unified agents map
        let agent = state.agents.get(&id).or_else(|| {
            let matches: Vec<_> = state.agents.iter().filter(|(k, _)| k.starts_with(&id)).collect();
            if matches.len() == 1 {
                Some(matches[0].1)
            } else {
                None
            }
        });

        if let Some(record) = agent {
            let path = agent_log_path(logs_path, &record.agent_id);
            let (content, new_offset) = read_log_file_with_offset(&path, lines, offset);
            return Response::AgentLogs {
                log_path: path,
                content,
                steps: vec![],
                offset: new_offset,
            };
        }

        // Fallback: search step_history across all jobs for matching agent_id
        for p in state.jobs.values() {
            for r in &p.step_history {
                if let Some(ref aid) = r.agent_id {
                    if *aid == id || aid.starts_with(&id) {
                        let path = agent_log_path(logs_path, aid);
                        let (content, new_offset) = read_log_file_with_offset(&path, lines, offset);
                        return Response::AgentLogs {
                            log_path: path,
                            content,
                            steps: vec![r.name.clone()],
                            offset: new_offset,
                        };
                    }
                }
            }
        }
    }

    let (content, steps, log_path, new_offset) = if let Some(step_name) = step {
        // Single step: find agent_id for that step
        let agent_id = job.and_then(|p| {
            p.step_history.iter().find(|r| r.name == step_name).and_then(|r| r.agent_id.clone())
        });

        if let Some(aid) = agent_id {
            let path = agent_log_path(logs_path, &aid);
            let (text, off) = read_log_file_with_offset(&path, lines, offset);
            (text, vec![step_name], path, off)
        } else {
            (String::new(), vec![step_name], logs_path.join("agent"), 0)
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

        // Multi-step doesn't support offset-based incremental reading
        (content, step_names, last_path, 0)
    };

    Response::AgentLogs { log_path, content, steps, offset: new_offset }
}

pub(super) fn handle_get_job_logs(
    id: String,
    lines: usize,
    offset: u64,
    state: &MaterializedState,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    logs_path: &Path,
) -> Response {
    // Resolve job ID (supports prefix matching), falling back to orphans
    let full_id = state
        .get_job(&id)
        .map(|p| p.id.clone())
        .or_else(|| super::query_orphans::find_orphan_id(orphans, &id))
        .unwrap_or(id);

    let log_path = job_log_path(logs_path, &full_id);
    let (content, new_offset) = read_log_file_with_offset(&log_path, lines, offset);
    Response::JobLogs { log_path, content, offset: new_offset }
}

pub(super) fn handle_get_worker_logs(
    name: String,
    project: String,
    lines: usize,
    offset: u64,
    project_path: Option<std::path::PathBuf>,
    state: &MaterializedState,
    logs_path: &Path,
) -> Response {
    let scoped = scoped_name(&project, &name);
    let log_path = worker_log_path(logs_path, &scoped);

    // If log exists, return it (worker was active at some point)
    if log_path.exists() {
        let (content, new_offset) = read_log_file_with_offset(&log_path, lines, offset);
        return Response::WorkerLogs { log_path, content, offset: new_offset };
    }

    // Log doesn't exist — check if worker is known
    let in_state = state.workers.contains_key(&scoped);
    let in_runbook = project_path.as_ref().is_some_and(|root| {
        oj_runbook::find_runbook_by_worker(&root.join(".oj/runbooks"), &name)
            .ok()
            .flatten()
            .is_some()
    });

    if in_state || in_runbook {
        // Worker exists but no logs yet
        Response::WorkerLogs { log_path, content: String::new(), offset: 0 }
    } else {
        // Worker not found — suggest
        let mut candidates: Vec<String> = state
            .workers
            .values()
            .filter(|w| w.project == project)
            .map(|w| w.name.clone())
            .collect();
        if let Some(ref root) = project_path {
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
            &project,
            "oj worker logs",
            state,
            suggest::ResourceType::Worker,
            &candidates,
        );

        if hint.is_empty() {
            Response::WorkerLogs { log_path, content: String::new(), offset: 0 }
        } else {
            Response::Error { message: format!("unknown worker: {}{}", name, hint) }
        }
    }
}

pub(super) fn handle_get_cron_logs(
    name: String,
    project: String,
    lines: usize,
    offset: u64,
    project_path: Option<std::path::PathBuf>,
    state: &MaterializedState,
    logs_path: &Path,
) -> Response {
    let scoped = scoped_name(&project, &name);
    let log_path = cron_log_path(logs_path, &scoped);

    // If log exists, return it
    if log_path.exists() {
        let (content, new_offset) = read_log_file_with_offset(&log_path, lines, offset);
        return Response::CronLogs { log_path, content, offset: new_offset };
    }

    // Log doesn't exist — check if cron is known
    let in_state = state.crons.values().any(|c| c.name == name);
    let in_runbook = project_path.as_ref().is_some_and(|root| {
        oj_runbook::find_runbook_by_cron(&root.join(".oj/runbooks"), &name).ok().flatten().is_some()
    });

    if in_state || in_runbook {
        Response::CronLogs { log_path, content: String::new(), offset: 0 }
    } else {
        // Cron not found — suggest
        let mut candidates: Vec<String> = state.crons.values().map(|c| c.name.clone()).collect();
        if let Some(ref root) = project_path {
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
            Response::CronLogs { log_path, content: String::new(), offset: 0 }
        } else {
            Response::Error { message: format!("unknown cron: {}{}", name, hint) }
        }
    }
}

pub(super) fn handle_get_queue_logs(
    queue: String,
    project: String,
    lines: usize,
    offset: u64,
    logs_path: &Path,
) -> Response {
    let scoped = scoped_name(&project, &queue);
    let path = queue_log_path(logs_path, &scoped);
    let (content, new_offset) = read_log_file_with_offset(&path, lines, offset);
    Response::QueueLogs { log_path: path, content, offset: new_offset }
}

/// Read a log file with optional offset-based incremental reading.
///
/// When `offset > 0`, reads only content after the byte offset (ignoring `lines`).
/// When `offset == 0`, returns the last N lines (or all if lines == 0).
/// Returns the content and the new byte offset (file size after reading).
fn read_log_file_with_offset(path: &Path, lines: usize, offset: u64) -> (String, u64) {
    if offset > 0 {
        // Incremental read from byte offset
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return (String::new(), offset),
        };
        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(_) => return (String::new(), offset),
        };
        let file_len = metadata.len();
        if file_len <= offset {
            return (String::new(), offset);
        }
        if file.seek(SeekFrom::Start(offset)).is_err() {
            return (String::new(), offset);
        }
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            return (String::new(), offset);
        }
        (buf, file_len)
    } else {
        // Full read with optional tail
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let file_len = text.len() as u64;
                let content = if lines > 0 {
                    let all_lines: Vec<&str> = text.lines().collect();
                    let start = all_lines.len().saturating_sub(lines);
                    all_lines[start..].join("\n")
                } else {
                    text
                };
                (content, file_len)
            }
            Err(_) => (String::new(), 0),
        }
    }
}
