// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Read-only query handlers.

#[path = "query_crons.rs"]
mod query_crons;
#[path = "query_orphans.rs"]
mod query_orphans;
#[path = "query_queues.rs"]
mod query_queues;

#[path = "query_projects.rs"]
mod query_projects;

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use oj_core::{scoped_name, split_scoped_name, StepOutcome};
use oj_storage::{MaterializedState, QueueItemStatus};

use oj_engine::breadcrumb::Breadcrumb;

use crate::protocol::{
    AgentDetail, AgentStatusEntry, AgentSummary, CronSummary, DecisionDetail, DecisionOptionDetail,
    DecisionSummary, NamespaceStatus, PipelineDetail, PipelineStatusEntry, PipelineSummary, Query,
    QueueItemSummary, QueueStatus, Response, SessionSummary, StepRecordDetail, WorkerSummary,
    WorkspaceDetail, WorkspaceSummary,
};

/// Handle query requests (read-only state access).
pub(super) fn handle_query(
    query: Query,
    state: &Arc<Mutex<MaterializedState>>,
    orphans: &Arc<Mutex<Vec<Breadcrumb>>>,
    logs_path: &Path,
    start_time: Instant,
) -> Response {
    match &query {
        Query::ListOrphans => return query_orphans::handle_list_orphans(orphans),
        Query::DismissOrphan { id } => {
            return query_orphans::handle_dismiss_orphan(orphans, id, logs_path)
        }
        Query::ListProjects => return query_projects::handle_list_projects(state),
        _ => {}
    }

    let state = state.lock();

    match query {
        Query::ListPipelines => {
            let mut pipelines: Vec<PipelineSummary> = state
                .pipelines
                .values()
                .map(|p| {
                    // updated_at_ms is the most recent activity timestamp from step history
                    let updated_at_ms = p
                        .step_history
                        .last()
                        .map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms))
                        .unwrap_or(0);
                    PipelineSummary {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        kind: p.kind.clone(),
                        step: p.step.clone(),
                        step_status: p.step_status.to_string(),
                        created_at_ms: p.step_history.first().map(|r| r.started_at_ms).unwrap_or(0),
                        updated_at_ms,
                        namespace: p.namespace.clone(),
                        retry_count: p.total_retries,
                    }
                })
                .collect();

            query_orphans::append_orphan_summaries(&mut pipelines, orphans);

            Response::Pipelines { pipelines }
        }

        Query::GetPipeline { id } => {
            let pipeline = state.get_pipeline(&id).map(|p| {
                let steps: Vec<StepRecordDetail> =
                    p.step_history.iter().map(StepRecordDetail::from).collect();

                // Compute agent summaries from log files
                let namespace = if p.namespace.is_empty() {
                    None
                } else {
                    Some(p.namespace.as_str())
                };
                let agents = compute_agent_summaries(&p.id, &steps, logs_path, namespace);

                Box::new(PipelineDetail {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    kind: p.kind.clone(),
                    step: p.step.clone(),
                    step_status: p.step_status.to_string(),
                    vars: p.vars.clone(),
                    workspace_path: p.workspace_path.clone(),
                    session_id: p.session_id.clone(),
                    error: p.error.clone(),
                    steps,
                    agents,
                    namespace: p.namespace.clone(),
                })
            });

            // If not found in state, check orphans
            let pipeline = pipeline.or_else(|| query_orphans::find_orphan_detail(orphans, &id));

            Response::Pipeline { pipeline }
        }

        Query::GetAgent { agent_id } => {
            // Search all pipelines for a matching agent by ID or prefix
            let agent = state.pipelines.values().find_map(|p| {
                let steps: Vec<StepRecordDetail> =
                    p.step_history.iter().map(StepRecordDetail::from).collect();

                let namespace = if p.namespace.is_empty() {
                    None
                } else {
                    Some(p.namespace.as_str())
                };
                let summaries = compute_agent_summaries(&p.id, &steps, logs_path, namespace);

                // Find agent matching by exact ID or prefix
                let summary = summaries
                    .iter()
                    .find(|a| a.agent_id == agent_id || a.agent_id.starts_with(&agent_id))?;

                // Find the matching step record for timestamps and error
                let step = steps
                    .iter()
                    .find(|s| s.agent_id.as_deref() == Some(&summary.agent_id));

                let error = step.and_then(|s| {
                    if s.outcome == "failed" {
                        s.detail.clone()
                    } else {
                        None
                    }
                });

                let started_at_ms = step.map(|s| s.started_at_ms).unwrap_or(0);
                let finished_at_ms = step.and_then(|s| s.finished_at_ms);

                Some(Box::new(AgentDetail {
                    agent_id: summary.agent_id.clone(),
                    agent_name: summary.agent_name.clone(),
                    pipeline_id: p.id.clone(),
                    pipeline_name: p.name.clone(),
                    step_name: summary.step_name.clone(),
                    namespace: namespace.map(|s| s.to_string()),
                    status: summary.status.clone(),
                    workspace_path: p.workspace_path.clone(),
                    session_id: p.session_id.clone(),
                    files_read: summary.files_read,
                    files_written: summary.files_written,
                    commands_run: summary.commands_run,
                    exit_reason: summary.exit_reason.clone(),
                    error,
                    started_at_ms,
                    finished_at_ms,
                    updated_at_ms: summary.updated_at_ms,
                }))
            });

            // If not found in pipelines, check standalone agent runs
            let agent = agent.or_else(|| {
                state.agent_runs.values().find_map(|ar| {
                    let ar_agent_id = ar.agent_id.as_deref().unwrap_or(&ar.id);
                    if ar_agent_id != agent_id
                        && !ar_agent_id.starts_with(&agent_id)
                        && ar.id != agent_id
                        && !ar.id.starts_with(&agent_id)
                    {
                        return None;
                    }
                    let namespace = if ar.namespace.is_empty() {
                        None
                    } else {
                        Some(ar.namespace.clone())
                    };
                    Some(Box::new(AgentDetail {
                        agent_id: ar_agent_id.to_string(),
                        agent_name: Some(ar.agent_name.clone()),
                        pipeline_id: String::new(),
                        pipeline_name: ar.command_name.clone(),
                        step_name: String::new(),
                        namespace,
                        status: format!("{}", ar.status),
                        workspace_path: Some(ar.cwd.clone()),
                        session_id: ar.session_id.clone(),
                        files_read: 0,
                        files_written: 0,
                        commands_run: 0,
                        exit_reason: ar.error.clone(),
                        error: ar.error.clone(),
                        started_at_ms: ar.created_at_ms,
                        finished_at_ms: None,
                        updated_at_ms: ar.updated_at_ms,
                    }))
                })
            });

            Response::Agent { agent }
        }

        Query::ListSessions => {
            let sessions = state
                .sessions
                .values()
                .map(|s| session_summary(s, &state))
                .collect();
            Response::Sessions { sessions }
        }

        Query::GetSession { id } => {
            let session = state
                .sessions
                .values()
                .find(|s| s.id == id || s.id.starts_with(&id))
                .map(|s| Box::new(session_summary(s, &state)));
            Response::Session { session }
        }

        Query::ListWorkspaces => {
            let workspaces = state
                .workspaces
                .values()
                .map(|w| {
                    let namespace = w
                        .owner
                        .as_deref()
                        .and_then(|owner| {
                            // Try pipeline first, then worker
                            state
                                .pipelines
                                .get(owner)
                                .map(|p| p.namespace.clone())
                                .or_else(|| state.workers.get(owner).map(|wr| wr.namespace.clone()))
                        })
                        .unwrap_or_default();
                    WorkspaceSummary {
                        id: w.id.clone(),
                        path: w.path.clone(),
                        branch: w.branch.clone(),
                        status: w.status.to_string(),
                        created_at_ms: w.created_at_ms,
                        namespace,
                    }
                })
                .collect();
            Response::Workspaces { workspaces }
        }

        Query::GetWorkspace { id } => {
            let workspace = state.workspaces.get(&id).map(|w| {
                Box::new(WorkspaceDetail {
                    id: w.id.clone(),
                    path: w.path.clone(),
                    branch: w.branch.clone(),
                    owner: w.owner.clone(),
                    status: w.status.to_string(),
                    created_at_ms: w.created_at_ms,
                })
            });
            Response::Workspace { workspace }
        }

        Query::GetAgentLogs { id, step, lines } => {
            use oj_engine::log_paths::agent_log_path;

            // Look up pipeline to find agent_ids from step history
            let pipeline = state.get_pipeline(&id);

            let (content, steps, log_path) = if let Some(step_name) = step {
                // Single step: find agent_id for that step
                let agent_id = pipeline.and_then(|p| {
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

                if let Some(p) = pipeline {
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

        Query::GetPipelineLogs { id, lines } => {
            use oj_engine::log_paths::pipeline_log_path;

            // Resolve pipeline ID (supports prefix matching), falling back to orphans
            let full_id = state
                .get_pipeline(&id)
                .map(|p| p.id.clone())
                .or_else(|| query_orphans::find_orphan_id(orphans, &id))
                .unwrap_or(id);

            let log_path = pipeline_log_path(logs_path, &full_id);
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
            Response::PipelineLogs { log_path, content }
        }

        Query::ListQueues {
            project_root,
            namespace,
        } => query_queues::list_queues(&state, &project_root, &namespace),

        Query::ListQueueItems {
            queue_name,
            namespace,
            project_root,
        } => {
            let key = scoped_name(&namespace, &queue_name);

            match state.queue_items.get(&key) {
                Some(queue_items) => {
                    let items = queue_items
                        .iter()
                        .map(|item| QueueItemSummary {
                            id: item.id.clone(),
                            status: format!("{:?}", item.status).to_lowercase(),
                            data: item.data.clone(),
                            worker_name: item.worker_name.clone(),
                            pushed_at_epoch_ms: item.pushed_at_epoch_ms,
                            failure_count: item.failure_count,
                        })
                        .collect();
                    Response::QueueItems { items }
                }
                None => {
                    // Queue not in state — check if it exists in runbooks
                    let in_runbook = project_root.as_ref().is_some_and(|root| {
                        oj_runbook::find_runbook_by_queue(&root.join(".oj/runbooks"), &queue_name)
                            .ok()
                            .flatten()
                            .is_some()
                    });
                    if in_runbook {
                        // Queue exists but has no items yet
                        Response::QueueItems { items: vec![] }
                    } else {
                        // Queue truly not found — suggest
                        use super::suggest;
                        let mut candidates: Vec<String> = state
                            .queue_items
                            .keys()
                            .filter_map(|k| {
                                let (ns, name) = split_scoped_name(k);
                                if ns == namespace {
                                    Some(name.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if let Some(ref root) = project_root {
                            let runbook_queues =
                                oj_runbook::collect_all_queues(&root.join(".oj/runbooks"))
                                    .unwrap_or_default();
                            for (name, _) in runbook_queues {
                                if !candidates.contains(&name) {
                                    candidates.push(name);
                                }
                            }
                        }

                        let hint = suggest::suggest_from_candidates(
                            &queue_name,
                            &namespace,
                            "oj queue show",
                            &state,
                            suggest::ResourceType::Queue,
                            &candidates,
                        );

                        if hint.is_empty() {
                            Response::QueueItems { items: vec![] }
                        } else {
                            Response::Error {
                                message: format!("unknown queue: {}{}", queue_name, hint),
                            }
                        }
                    }
                }
            }
        }

        Query::GetAgentSignal { agent_id } => {
            // Check standalone agent runs first
            let agent_run_match = state
                .agent_runs
                .values()
                .find(|ar| ar.agent_id.as_deref() == Some(&agent_id));
            if let Some(ar) = agent_run_match {
                if let Some(s) = &ar.action_tracker.agent_signal {
                    return Response::AgentSignal {
                        signaled: true,
                        kind: Some(s.kind.clone()),
                        message: s.message.clone(),
                    };
                }
                // Agent run exists but no signal — don't allow exit
                return Response::AgentSignal {
                    signaled: false,
                    kind: None,
                    message: None,
                };
            }

            // Find pipeline by agent_id in current step and return its signal
            let pipeline_signal = state.pipelines.values().find_map(|p| {
                let matches = p
                    .step_history
                    .iter()
                    .rfind(|r| r.name == p.step)
                    .and_then(|r| r.agent_id.as_deref())
                    == Some(&agent_id);
                if matches {
                    Some(p.action_tracker.agent_signal.as_ref())
                } else {
                    None
                }
            });

            match pipeline_signal {
                Some(Some(s)) => Response::AgentSignal {
                    signaled: true,
                    kind: Some(s.kind.clone()),
                    message: s.message.clone(),
                },
                Some(None) => Response::AgentSignal {
                    signaled: false,
                    kind: None,
                    message: None,
                },
                None => {
                    // No pipeline or agent_run owns this agent — orphaned or pipeline advanced.
                    // Allow exit to prevent the agent from getting stuck.
                    Response::AgentSignal {
                        signaled: true,
                        kind: None,
                        message: None,
                    }
                }
            }
        }

        Query::ListAgents {
            pipeline_id,
            status,
        } => {
            let mut agents: Vec<AgentSummary> = Vec::new();

            for p in state.pipelines.values() {
                if let Some(ref prefix) = pipeline_id {
                    if !p.id.starts_with(prefix.as_str()) {
                        continue;
                    }
                }

                let steps: Vec<StepRecordDetail> =
                    p.step_history.iter().map(StepRecordDetail::from).collect();

                let namespace = if p.namespace.is_empty() {
                    None
                } else {
                    Some(p.namespace.clone())
                };
                let mut summaries =
                    compute_agent_summaries(&p.id, &steps, logs_path, namespace.as_deref());

                if let Some(ref s) = status {
                    summaries.retain(|a| a.status == *s);
                }

                agents.extend(summaries);
            }

            // Include standalone agent runs
            for ar in state.agent_runs.values() {
                let ar_status = format!("{}", ar.status);
                if let Some(ref s) = status {
                    if ar_status != *s {
                        continue;
                    }
                }
                let namespace = if ar.namespace.is_empty() {
                    None
                } else {
                    Some(ar.namespace.clone())
                };
                agents.push(AgentSummary {
                    pipeline_id: String::new(),
                    step_name: String::new(),
                    agent_id: ar.agent_id.clone().unwrap_or_else(|| ar.id.clone()),
                    agent_name: Some(ar.agent_name.clone()),
                    namespace,
                    status: ar_status,
                    files_read: 0,
                    files_written: 0,
                    commands_run: 0,
                    exit_reason: ar.error.clone(),
                    updated_at_ms: ar.updated_at_ms,
                });
            }

            // Sort by most recently updated first
            agents.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

            Response::Agents { agents }
        }

        Query::GetWorkerLogs {
            name,
            namespace,
            lines,
            project_root,
        } => {
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
                use super::suggest;
                let mut candidates: Vec<String> = state
                    .workers
                    .values()
                    .filter(|w| w.namespace == namespace)
                    .map(|w| w.name.clone())
                    .collect();
                if let Some(ref root) = project_root {
                    let runbook_workers =
                        oj_runbook::collect_all_workers(&root.join(".oj/runbooks"))
                            .unwrap_or_default();
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
                    &state,
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

        Query::ListWorkers => {
            let workers = state
                .workers
                .values()
                .map(|w| {
                    // Derive updated_at_ms from the most recently updated active pipeline
                    let updated_at_ms = w
                        .active_pipeline_ids
                        .iter()
                        .filter_map(|pid| state.pipelines.get(pid))
                        .filter_map(|p| {
                            p.step_history
                                .last()
                                .map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms))
                        })
                        .max()
                        .unwrap_or(0);
                    WorkerSummary {
                        name: w.name.clone(),
                        namespace: w.namespace.clone(),
                        queue: w.queue_name.clone(),
                        status: w.status.clone(),
                        active: w.active_pipeline_ids.len(),
                        concurrency: w.concurrency,
                        updated_at_ms,
                    }
                })
                .collect();
            Response::Workers { workers }
        }

        Query::GetCronLogs {
            name,
            namespace,
            lines,
            project_root,
        } => {
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
                use super::suggest;
                let mut candidates: Vec<String> =
                    state.crons.values().map(|c| c.name.clone()).collect();
                if let Some(ref root) = project_root {
                    let runbook_crons = oj_runbook::collect_all_crons(&root.join(".oj/runbooks"))
                        .unwrap_or_default();
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
                    &state,
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

        Query::ListCrons => {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let crons = state
                .crons
                .values()
                .map(|c| {
                    let time = query_crons::cron_time_display(c, now_ms);
                    // Use run_target if available, fall back to pipeline_name
                    let target = if !c.run_target.is_empty() {
                        c.run_target.clone()
                    } else {
                        c.pipeline_name.clone()
                    };
                    CronSummary {
                        name: c.name.clone(),
                        namespace: c.namespace.clone(),
                        interval: c.interval.clone(),
                        pipeline: target,
                        status: c.status.clone(),
                        time,
                    }
                })
                .collect();
            Response::Crons { crons }
        }

        Query::StatusOverview => {
            let uptime_secs = start_time.elapsed().as_secs();
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            // Collect all namespaces seen across entities
            let mut ns_active: BTreeMap<String, Vec<PipelineStatusEntry>> = BTreeMap::new();
            let mut ns_escalated: BTreeMap<String, Vec<PipelineStatusEntry>> = BTreeMap::new();
            let mut ns_agents: BTreeMap<String, Vec<AgentStatusEntry>> = BTreeMap::new();

            for p in state.pipelines.values() {
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

                let entry = PipelineStatusEntry {
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

            // Collect standalone agents
            for ar in state.agent_runs.values() {
                if ar.status.is_terminal() {
                    continue;
                }
                let agent_id = ar.agent_id.clone().unwrap_or_else(|| ar.id.clone());
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
                        active: w.active_pipeline_ids.len(),
                        concurrency: w.concurrency,
                        updated_at_ms: w
                            .active_pipeline_ids
                            .iter()
                            .filter_map(|pid| state.pipelines.get(pid))
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

            // Collect orphaned pipelines grouped by namespace
            let mut ns_orphaned = query_orphans::collect_orphan_status_entries(orphans, now_ms);

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
                    active_pipelines: ns_active.remove(&ns).unwrap_or_default(),
                    escalated_pipelines: ns_escalated.remove(&ns).unwrap_or_default(),
                    orphaned_pipelines: ns_orphaned.remove(&ns).unwrap_or_default(),
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

        Query::GetQueueLogs {
            queue_name,
            namespace,
            lines,
        } => {
            use oj_engine::log_paths::queue_log_path;

            let scoped = scoped_name(&namespace, &queue_name);
            let path = queue_log_path(logs_path, &scoped);
            let content = read_log_file(&path, lines);
            Response::QueueLogs {
                log_path: path,
                content,
            }
        }

        Query::ListDecisions { namespace: _ } => {
            let mut decisions: Vec<DecisionSummary> = state
                .decisions
                .values()
                .filter(|d| !d.is_resolved())
                .map(|d| {
                    let pipeline_name = state
                        .pipelines
                        .get(&d.pipeline_id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    let summary = if d.context.len() > 80 {
                        format!("{}...", &d.context[..77])
                    } else {
                        d.context.clone()
                    };
                    DecisionSummary {
                        id: d.id.to_string(),
                        pipeline_id: d.pipeline_id.clone(),
                        pipeline_name,
                        source: format!("{:?}", d.source).to_lowercase(),
                        summary,
                        created_at_ms: d.created_at_ms,
                        namespace: d.namespace.clone(),
                    }
                })
                .collect();
            decisions.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
            Response::Decisions { decisions }
        }

        Query::GetDecision { id } => {
            let decision = state.get_decision(&id).map(|d| {
                let pipeline_name = state
                    .pipelines
                    .get(&d.pipeline_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let options = d
                    .options
                    .iter()
                    .enumerate()
                    .map(|(i, opt)| DecisionOptionDetail {
                        number: i + 1,
                        label: opt.label.clone(),
                        description: opt.description.clone(),
                        recommended: opt.recommended,
                    })
                    .collect();
                Box::new(DecisionDetail {
                    id: d.id.to_string(),
                    pipeline_id: d.pipeline_id.clone(),
                    pipeline_name,
                    agent_id: d.agent_id.clone(),
                    source: format!("{:?}", d.source).to_lowercase(),
                    context: d.context.clone(),
                    options,
                    chosen: d.chosen,
                    message: d.message.clone(),
                    created_at_ms: d.created_at_ms,
                    resolved_at_ms: d.resolved_at_ms,
                    namespace: d.namespace.clone(),
                })
            });
            Response::Decision { decision }
        }

        // Handled by early return above; included for exhaustiveness
        Query::ListOrphans | Query::DismissOrphan { .. } | Query::ListProjects => unreachable!(),
    }
}

/// Build a `SessionSummary` from a stored session, deriving fields from its pipeline.
fn session_summary(s: &oj_storage::Session, state: &MaterializedState) -> SessionSummary {
    let updated_at_ms = state
        .pipelines
        .get(&s.pipeline_id)
        .and_then(|p| {
            p.step_history
                .last()
                .map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms))
        })
        .unwrap_or(0);
    let namespace = state
        .pipelines
        .get(&s.pipeline_id)
        .map(|p| p.namespace.clone())
        .unwrap_or_default();
    SessionSummary {
        id: s.id.clone(),
        namespace,
        pipeline_id: Some(s.pipeline_id.clone()),
        updated_at_ms,
    }
}

/// Compute agent summaries from step records by scanning agent log files.
fn compute_agent_summaries(
    pipeline_id: &str,
    steps: &[StepRecordDetail],
    logs_path: &Path,
    namespace: Option<&str>,
) -> Vec<AgentSummary> {
    use oj_engine::log_paths::agent_log_path;

    steps
        .iter()
        .filter_map(|step| {
            let agent_id = step.agent_id.as_ref()?;
            let log_path = agent_log_path(logs_path, agent_id);

            let content = std::fs::read_to_string(&log_path).unwrap_or_default();

            let mut files_read = 0usize;
            let mut files_written = 0usize;
            let mut commands_run = 0usize;

            for line in content.lines() {
                // Lines are formatted as: "TIMESTAMP kind: details"
                // Find the kind prefix after the timestamp
                let rest = match line.find(' ') {
                    Some(pos) => &line[pos + 1..],
                    None => continue,
                };

                if rest.starts_with("read:") {
                    files_read += 1;
                } else if rest.starts_with("wrote:") || rest.starts_with("edited:") {
                    files_written += 1;
                } else if rest.starts_with("bash:") {
                    commands_run += 1;
                }
            }

            // Determine exit reason from step outcome
            let exit_reason = match step.outcome.as_str() {
                "completed" => Some("completed".to_string()),
                "waiting" => Some("idle".to_string()),
                "failed" => step
                    .detail
                    .as_ref()
                    .map(|d| format!("failed: {}", d))
                    .or(Some("failed".to_string())),
                "running" => None,
                _ => None,
            };

            // Check for "session gone" in log
            let exit_reason = if content.contains("error: session") {
                Some("gone".to_string())
            } else {
                exit_reason
            };

            let updated_at_ms = step.finished_at_ms.unwrap_or(step.started_at_ms);

            Some(AgentSummary {
                pipeline_id: pipeline_id.to_string(),
                step_name: step.name.clone(),
                agent_id: agent_id.clone(),
                agent_name: step.agent_name.clone(),
                namespace: namespace.map(|s| s.to_string()),
                status: step.outcome.clone(),
                files_read,
                files_written,
                commands_run,
                exit_reason,
                updated_at_ms,
            })
        })
        .collect()
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

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
