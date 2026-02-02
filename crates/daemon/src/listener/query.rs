// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Read-only query handlers.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::StepOutcome;
use oj_storage::MaterializedState;

use crate::protocol::{
    AgentSummary, PipelineDetail, PipelineSummary, Query, QueueItemSummary, Response,
    SessionSummary, StepRecordDetail, WorkerSummary, WorkspaceDetail, WorkspaceSummary,
};

/// Handle query requests (read-only state access).
pub(super) fn handle_query(
    query: Query,
    state: &Arc<Mutex<MaterializedState>>,
    logs_path: &Path,
) -> Response {
    let state = state.lock();

    match query {
        Query::ListPipelines => {
            let pipelines = state
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
                    }
                })
                .collect();
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
                    })
                    .collect();

                // Compute agent summaries from log files
                let agents = compute_agent_summaries(&p.id, &steps, logs_path);

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
            Response::Pipeline { pipeline }
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
                    SessionSummary {
                        id: s.id.clone(),
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
                .map(|w| WorkspaceSummary {
                    id: w.id.clone(),
                    path: w.path.clone(),
                    branch: w.branch.clone(),
                    status: w.status.to_string(),
                    created_at_ms: w.created_at_ms,
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
            // Resolve pipeline ID (supports prefix matching)
            let full_id = state.get_pipeline(&id).map(|p| p.id.clone()).unwrap_or(id);

            let log_path = logs_path.join(format!("{}.log", full_id));
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
                    })
                    .collect();

                let mut summaries = compute_agent_summaries(&p.id, &steps, logs_path);

                if let Some(ref s) = status {
                    summaries.retain(|a| a.status == *s);
                }

                agents.extend(summaries);
            }

            // Sort: running agents first, then by pipeline_id (most recent first)
            agents.sort_by(|a, b| {
                let a_running = a.status == "running";
                let b_running = b.status == "running";
                b_running
                    .cmp(&a_running)
                    .then_with(|| b.pipeline_id.cmp(&a.pipeline_id))
            });

            Response::Agents { agents }
        }

        Query::ListWorkers => {
            let workers = state
                .workers
                .values()
                .map(|w| WorkerSummary {
                    name: w.name.clone(),
                    namespace: w.namespace.clone(),
                    queue: w.queue_name.clone(),
                    status: w.status.clone(),
                    active: w.active_pipeline_ids.len(),
                    concurrency: w.concurrency,
                })
                .collect();
            Response::Workers { workers }
        }
    }
}

/// Compute agent summaries from step records by scanning agent log files.
fn compute_agent_summaries(
    pipeline_id: &str,
    steps: &[StepRecordDetail],
    logs_path: &Path,
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

            Some(AgentSummary {
                pipeline_id: pipeline_id.to_string(),
                step_name: step.name.clone(),
                agent_id: agent_id.clone(),
                status: step.outcome.clone(),
                files_read,
                files_written,
                commands_run,
                exit_reason,
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
