// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent-related query handlers.

use std::collections::HashSet;
use std::path::Path;

use oj_core::{OwnerId, StepOutcome};
use oj_storage::MaterializedState;

use crate::protocol::{AgentDetail, AgentSummary, Response, StepRecordDetail};

pub(super) fn handle_get_agent(
    agent_id: String,
    state: &MaterializedState,
    logs_path: &Path,
) -> Response {
    // Search all jobs for a matching agent by ID or prefix
    let agent = state.jobs.values().find_map(|p| {
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
            job_id: p.id.clone(),
            job_name: p.name.clone(),
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

    // If not found in jobs, check standalone agent runs
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
                job_id: String::new(),
                job_name: ar.command_name.clone(),
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

pub(super) fn handle_get_agent_signal(agent_id: String, state: &MaterializedState) -> Response {
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

    // Find job by agent_id in current step and return its signal
    let job_signal = state.jobs.values().find_map(|p| {
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

    match job_signal {
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
            // No job or agent_run owns this agent — orphaned or job advanced.
            // Allow exit to prevent the agent from getting stuck.
            Response::AgentSignal {
                signaled: true,
                kind: None,
                message: None,
            }
        }
    }
}

pub(super) fn handle_list_agents(
    job_id: Option<String>,
    status: Option<String>,
    state: &MaterializedState,
    logs_path: &Path,
) -> Response {
    let mut agents: Vec<AgentSummary> = Vec::new();
    let mut tracked_agent_ids: HashSet<String> = HashSet::new();

    // Primary source: unified agents map
    for record in state.agents.values() {
        // Apply job_id filter (only matches Job-owned agents)
        if let Some(ref prefix) = job_id {
            match &record.owner {
                OwnerId::Job(jid) if jid.as_str().starts_with(prefix.as_str()) => {}
                OwnerId::Job(_) => continue,
                OwnerId::AgentRun(_) => continue,
            }
        }

        let status_str = match record.status {
            oj_core::AgentRecordStatus::Starting => "running",
            oj_core::AgentRecordStatus::Running => "running",
            oj_core::AgentRecordStatus::Idle => "waiting",
            oj_core::AgentRecordStatus::Exited => "completed",
            oj_core::AgentRecordStatus::Gone => "failed",
        };

        if let Some(ref s) = status {
            if status_str != s.as_str() {
                continue;
            }
        }

        // Derive job_id and step_name from owner
        let (owner_job_id, step_name) = match &record.owner {
            OwnerId::Job(jid) => {
                let sname = state
                    .jobs
                    .get(jid.as_str())
                    .and_then(|j| {
                        j.step_history
                            .iter()
                            .find(|r| r.agent_id.as_deref() == Some(&record.agent_id))
                            .map(|r| r.name.clone())
                    })
                    .unwrap_or_default();
                (jid.to_string(), sname)
            }
            OwnerId::AgentRun(_) => (String::new(), String::new()),
        };

        let namespace = if record.namespace.is_empty() {
            None
        } else {
            Some(record.namespace.clone())
        };

        // Read agent log for file stats
        let (files_read, files_written, commands_run) =
            count_agent_log_stats(logs_path, &record.agent_id);

        // Derive exit_reason
        let exit_reason = match &record.owner {
            OwnerId::Job(jid) => state.jobs.get(jid.as_str()).and_then(|j| {
                j.step_history
                    .iter()
                    .find(|r| r.agent_id.as_deref() == Some(&record.agent_id))
                    .and_then(|r| match &r.outcome {
                        StepOutcome::Completed => Some("completed".to_string()),
                        StepOutcome::Waiting(reason) => Some(format!("idle: {}", reason)),
                        StepOutcome::Failed(msg) => Some(format!("failed: {}", msg)),
                        _ => None,
                    })
            }),
            OwnerId::AgentRun(arid) => state
                .agent_runs
                .get(arid.as_str())
                .and_then(|ar| ar.error.clone()),
        };

        tracked_agent_ids.insert(record.agent_id.clone());

        agents.push(AgentSummary {
            job_id: owner_job_id,
            step_name,
            agent_id: record.agent_id.clone(),
            agent_name: Some(record.agent_name.clone()),
            namespace,
            status: status_str.to_string(),
            files_read,
            files_written,
            commands_run,
            exit_reason,
            updated_at_ms: record.updated_at_ms,
        });
    }

    // Fallback: job step_history for agents not in agents map (old WAL entries)
    for p in state.jobs.values() {
        if let Some(ref prefix) = job_id {
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
        let mut summaries = compute_agent_summaries(&p.id, &steps, logs_path, namespace.as_deref());

        // Skip agents already tracked from the agents map
        summaries.retain(|a| !tracked_agent_ids.contains(&a.agent_id));

        if let Some(ref s) = status {
            summaries.retain(|a| a.status == *s);
        }

        agents.extend(summaries);
    }

    // Fallback: standalone agent runs not in agents map
    for ar in state.agent_runs.values() {
        let aid = ar.agent_id.clone().unwrap_or_else(|| ar.id.clone());
        if tracked_agent_ids.contains(&aid) {
            continue;
        }

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
            job_id: String::new(),
            step_name: String::new(),
            agent_id: aid,
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

/// Count file read/write/command stats from an agent log file.
fn count_agent_log_stats(logs_path: &Path, agent_id: &str) -> (usize, usize, usize) {
    use oj_engine::log_paths::agent_log_path;

    let log_path = agent_log_path(logs_path, agent_id);
    let content = std::fs::read_to_string(&log_path).unwrap_or_default();

    let mut files_read = 0usize;
    let mut files_written = 0usize;
    let mut commands_run = 0usize;

    for line in content.lines() {
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

    (files_read, files_written, commands_run)
}

/// Compute agent summaries from step records by scanning agent log files.
pub(super) fn compute_agent_summaries(
    job_id: &str,
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
                job_id: job_id.to_string(),
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
