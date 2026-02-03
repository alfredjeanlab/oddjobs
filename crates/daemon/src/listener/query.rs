// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Read-only query handlers.

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

use oj_core::{StepOutcome, StepStatus};
use oj_storage::{MaterializedState, QueueItemStatus};

use oj_engine::breadcrumb::Breadcrumb;

use crate::protocol::{
    AgentDetail, AgentStatusEntry, AgentSummary, CronSummary, NamespaceStatus, PipelineDetail,
    PipelineStatusEntry, PipelineSummary, Query, QueueItemSummary, QueueStatus, Response,
    SessionSummary, StepRecordDetail, WorkerSummary, WorkspaceDetail, WorkspaceSummary,
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
                        step_status: format!("{:?}", p.step_status),
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
                let steps: Vec<StepRecordDetail> = p
                    .step_history
                    .iter()
                    .map(|r| StepRecordDetail {
                        name: r.name.clone(),
                        started_at_ms: r.started_at_ms,
                        finished_at_ms: r.finished_at_ms,
                        outcome: match &r.outcome {
                            StepOutcome::Running => "running".to_string(),
                            StepOutcome::Completed => "completed".to_string(),
                            StepOutcome::Failed(_) => "failed".to_string(),
                            StepOutcome::Waiting(_) => "waiting".to_string(),
                        },
                        detail: match &r.outcome {
                            StepOutcome::Failed(e) => Some(e.clone()),
                            StepOutcome::Waiting(r) => Some(r.clone()),
                            _ => None,
                        },
                        agent_id: r.agent_id.clone(),
                        agent_name: r.agent_name.clone(),
                    })
                    .collect();

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
                    step_status: format!("{:?}", p.step_status),
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
                let steps: Vec<StepRecordDetail> = p
                    .step_history
                    .iter()
                    .map(|r| StepRecordDetail {
                        name: r.name.clone(),
                        started_at_ms: r.started_at_ms,
                        finished_at_ms: r.finished_at_ms,
                        outcome: match &r.outcome {
                            StepOutcome::Running => "running".to_string(),
                            StepOutcome::Completed => "completed".to_string(),
                            StepOutcome::Failed(_) => "failed".to_string(),
                            StepOutcome::Waiting(_) => "waiting".to_string(),
                        },
                        detail: match &r.outcome {
                            StepOutcome::Failed(e) => Some(e.clone()),
                            StepOutcome::Waiting(r) => Some(r.clone()),
                            _ => None,
                        },
                        agent_id: r.agent_id.clone(),
                        agent_name: r.agent_name.clone(),
                    })
                    .collect();

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

            Response::Agent { agent }
        }

        Query::ListSessions => {
            let sessions = state
                .sessions
                .values()
                .map(|s| {
                    // Derive updated_at_ms from associated pipeline's step history
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
                })
                .collect();
            Response::Sessions { sessions }
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

            // Resolve pipeline ID (supports prefix matching)
            let full_id = state.get_pipeline(&id).map(|p| p.id.clone()).unwrap_or(id);

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
        } => {
            // Use scoped key: namespace/queue_name (matching storage::state::scoped_key)
            let key = if namespace.is_empty() {
                queue_name.clone()
            } else {
                format!("{}/{}", namespace, queue_name)
            };
            let items = state
                .queue_items
                .get(&key)
                .map(|queue_items| {
                    queue_items
                        .iter()
                        .map(|item| QueueItemSummary {
                            id: item.id.clone(),
                            status: format!("{:?}", item.status).to_lowercase(),
                            data: item.data.clone(),
                            worker_name: item.worker_name.clone(),
                            pushed_at_epoch_ms: item.pushed_at_epoch_ms,
                            failure_count: item.failure_count,
                        })
                        .collect()
                })
                .unwrap_or_default();
            Response::QueueItems { items }
        }

        Query::GetAgentSignal { agent_id } => {
            // Find pipeline by agent_id in current step and return its signal
            let signal = state.pipelines.values().find_map(|p| {
                (p.step_history.last()?.agent_id.as_deref() == Some(&agent_id))
                    .then_some(p.agent_signal.as_ref())?
            });
            signal.map_or(
                Response::AgentSignal {
                    signaled: false,
                    kind: None,
                    message: None,
                },
                |s| Response::AgentSignal {
                    signaled: true,
                    kind: Some(s.kind.clone()),
                    message: s.message.clone(),
                },
            )
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

                let steps: Vec<StepRecordDetail> = p
                    .step_history
                    .iter()
                    .map(|r| StepRecordDetail {
                        name: r.name.clone(),
                        started_at_ms: r.started_at_ms,
                        finished_at_ms: r.finished_at_ms,
                        outcome: match &r.outcome {
                            StepOutcome::Running => "running".to_string(),
                            StepOutcome::Completed => "completed".to_string(),
                            StepOutcome::Failed(_) => "failed".to_string(),
                            StepOutcome::Waiting(_) => "waiting".to_string(),
                        },
                        detail: match &r.outcome {
                            StepOutcome::Failed(e) => Some(e.clone()),
                            StepOutcome::Waiting(r) => Some(r.clone()),
                            _ => None,
                        },
                        agent_id: r.agent_id.clone(),
                        agent_name: r.agent_name.clone(),
                    })
                    .collect();

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

            // Sort by most recently updated first
            agents.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

            Response::Agents { agents }
        }

        Query::GetWorkerLogs {
            name,
            namespace,
            lines,
        } => {
            use oj_engine::log_paths::worker_log_path;

            let scoped_name = if namespace.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", namespace, name)
            };

            let log_path = worker_log_path(logs_path, &scoped_name);
            let content = read_log_file(&log_path, lines);
            Response::WorkerLogs { log_path, content }
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

        Query::GetCronLogs { name, lines } => {
            use oj_engine::log_paths::cron_log_path;

            let log_path = cron_log_path(logs_path, &name);
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
            Response::CronLogs { log_path, content }
        }

        Query::ListCrons => {
            let crons = state
                .crons
                .values()
                .map(|c| CronSummary {
                    name: c.name.clone(),
                    namespace: c.namespace.clone(),
                    interval: c.interval.clone(),
                    pipeline: c.pipeline_name.clone(),
                    status: c.status.clone(),
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

                let waiting_reason = match p.step_history.last().map(|r| &r.outcome) {
                    Some(StepOutcome::Waiting(reason)) => Some(reason.clone()),
                    _ => None,
                };

                let entry = PipelineStatusEntry {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    kind: p.kind.clone(),
                    step: p.step.clone(),
                    step_status: format!("{:?}", p.step_status),
                    elapsed_ms,
                    waiting_reason,
                };

                let ns = p.namespace.clone();
                match p.step_status {
                    StepStatus::Waiting => ns_escalated.entry(ns).or_default().push(entry),
                    _ => ns_active.entry(ns).or_default().push(entry),
                }

                // Extract active agents from this pipeline's step history
                if let Some(last_step) = p.step_history.last() {
                    if let Some(ref agent_id) = last_step.agent_id {
                        let status = match &last_step.outcome {
                            StepOutcome::Running => "running",
                            StepOutcome::Waiting(_) => "waiting",
                            _ => continue,
                        };
                        ns_agents
                            .entry(p.namespace.clone())
                            .or_default()
                            .push(AgentStatusEntry {
                                agent_id: agent_id.clone(),
                                pipeline_name: p.name.clone(),
                                step_name: last_step.name.clone(),
                                status: status.to_string(),
                            });
                    }
                }
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
                let (ns, queue_name) = query_queues::parse_scoped_key(scoped_key);

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

                ns_queues.entry(ns).or_default().push(QueueStatus {
                    name: queue_name,
                    pending,
                    active,
                    dead,
                });
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

            let mut namespaces: Vec<NamespaceStatus> = all_namespaces
                .into_iter()
                .map(|ns| NamespaceStatus {
                    active_pipelines: ns_active.remove(&ns).unwrap_or_default(),
                    escalated_pipelines: ns_escalated.remove(&ns).unwrap_or_default(),
                    orphaned_pipelines: ns_orphaned.remove(&ns).unwrap_or_default(),
                    workers: ns_workers.remove(&ns).unwrap_or_default(),
                    queues: ns_queues.remove(&ns).unwrap_or_default(),
                    active_agents: ns_agents.remove(&ns).unwrap_or_default(),
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

            let scoped = if namespace.is_empty() {
                queue_name
            } else {
                format!("{}/{}", namespace, queue_name)
            };
            let path = queue_log_path(logs_path, &scoped);
            let content = read_log_file(&path, lines);
            Response::QueueLogs {
                log_path: path,
                content,
            }
        }

        // Handled by early return above; included for exhaustiveness
        Query::ListOrphans | Query::DismissOrphan { .. } | Query::ListProjects => unreachable!(),
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
