// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Read-only query handlers.

#[path = "query_agents.rs"]
mod query_agents;
#[path = "query_crons.rs"]
mod query_crons;
#[path = "query_logs.rs"]
mod query_logs;
#[path = "query_orphans.rs"]
mod query_orphans;
#[path = "query_projects.rs"]
mod query_projects;
#[path = "query_queues.rs"]
mod query_queues;
#[path = "query_status.rs"]
mod query_status;

use std::time::{SystemTime, UNIX_EPOCH};

use oj_core::{namespace_to_option, scoped_name, split_scoped_name, StepStatusKind};
use oj_storage::MaterializedState;

use crate::protocol::{
    CronSummary, DecisionDetail, DecisionOptionDetail, DecisionSummary, JobDetail, JobSummary,
    Query, QueueItemSummary, Response, SessionSummary, StepRecordDetail, WorkerSummary,
    WorkspaceDetail, WorkspaceSummary,
};

use super::ListenCtx;

/// Handle query requests (read-only state access).
pub(super) fn handle_query(ctx: &ListenCtx, query: Query) -> Response {
    match &query {
        Query::ListOrphans => return query_orphans::handle_list_orphans(&ctx.orphans),
        Query::DismissOrphan { id } => {
            return query_orphans::handle_dismiss_orphan(&ctx.orphans, id, &ctx.logs_path)
        }
        Query::ListProjects => return query_projects::handle_list_projects(&ctx.state),
        _ => {}
    }

    let state = ctx.state.lock();

    match query {
        Query::ListJobs => {
            let mut jobs: Vec<JobSummary> = state
                .jobs
                .values()
                .map(|p| {
                    // updated_at_ms is the most recent activity timestamp from step history
                    let updated_at_ms = p
                        .step_history
                        .last()
                        .map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms))
                        .unwrap_or(0);
                    JobSummary {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        kind: p.kind.clone(),
                        step: p.step.clone(),
                        step_status: StepStatusKind::from(&p.step_status),
                        created_at_ms: p.step_history.first().map(|r| r.started_at_ms).unwrap_or(0),
                        updated_at_ms,
                        namespace: p.namespace.clone(),
                        retry_count: p.total_retries,
                    }
                })
                .collect();

            query_orphans::append_orphan_summaries(&mut jobs, &ctx.orphans);

            Response::Jobs { jobs }
        }

        Query::GetJob { id } => {
            let job = state.get_job(&id).map(|p| {
                let steps: Vec<StepRecordDetail> =
                    p.step_history.iter().map(StepRecordDetail::from).collect();

                // Compute agent summaries from log files
                let namespace = namespace_to_option(&p.namespace);
                let agents =
                    query_agents::compute_agent_summaries(&p.id, &steps, &ctx.logs_path, namespace);

                // Filter variables to only show declared scope prefixes
                // System variables (agent_id, job_id, prompt, etc.) are excluded
                let vars = filter_vars_by_scope(&p.vars);

                Box::new(JobDetail {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    kind: p.kind.clone(),
                    step: p.step.clone(),
                    step_status: StepStatusKind::from(&p.step_status),
                    vars,
                    workspace_path: p.workspace_path.clone(),
                    session_id: p.session_id.clone(),
                    error: p.error.clone(),
                    steps,
                    agents,
                    namespace: p.namespace.clone(),
                })
            });

            // If not found in state, check orphans
            let job = job.or_else(|| query_orphans::find_orphan_detail(&ctx.orphans, &id));

            Response::Job { job }
        }

        Query::GetAgent { agent_id } => {
            query_agents::handle_get_agent(agent_id, &state, &ctx.logs_path)
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
                    let namespace = match &w.owner {
                        Some(oj_core::OwnerId::Job(job_id)) => state
                            .jobs
                            .get(job_id.as_str())
                            .map(|p| p.namespace.clone())
                            .unwrap_or_default(),
                        Some(oj_core::OwnerId::AgentRun(ar_id)) => state
                            .agent_runs
                            .get(ar_id.as_str())
                            .map(|ar| ar.namespace.clone())
                            .unwrap_or_default(),
                        None => String::new(),
                    };
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
                let owner_str = w.owner.as_ref().map(|o| match o {
                    oj_core::OwnerId::Job(job_id) => job_id.to_string(),
                    oj_core::OwnerId::AgentRun(ar_id) => ar_id.to_string(),
                });
                Box::new(WorkspaceDetail {
                    id: w.id.clone(),
                    path: w.path.clone(),
                    branch: w.branch.clone(),
                    owner: owner_str,
                    status: w.status.to_string(),
                    created_at_ms: w.created_at_ms,
                })
            });
            Response::Workspace { workspace }
        }

        Query::GetAgentLogs { id, step, lines } => {
            query_logs::handle_get_agent_logs(id, step, lines, &state, &ctx.logs_path)
        }

        Query::GetJobLogs { id, lines } => {
            query_logs::handle_get_job_logs(id, lines, &state, &ctx.orphans, &ctx.logs_path)
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
                            status: item.status.to_string(),
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
            query_agents::handle_get_agent_signal(agent_id, &state)
        }

        Query::ListAgents { job_id, status } => {
            query_agents::handle_list_agents(job_id, status, &state, &ctx.logs_path)
        }

        Query::GetWorkerLogs {
            name,
            namespace,
            lines,
            project_root,
        } => query_logs::handle_get_worker_logs(
            name,
            namespace,
            lines,
            project_root,
            &state,
            &ctx.logs_path,
        ),

        Query::ListWorkers => {
            let workers = state
                .workers
                .values()
                .map(|w| {
                    // Derive updated_at_ms from the most recently updated active job
                    let updated_at_ms = w
                        .active_job_ids
                        .iter()
                        .filter_map(|pid| state.jobs.get(pid))
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
                        active: w.active_job_ids.len(),
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
        } => query_logs::handle_get_cron_logs(
            name,
            namespace,
            lines,
            project_root,
            &state,
            &ctx.logs_path,
        ),

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
                    CronSummary {
                        name: c.name.clone(),
                        namespace: c.namespace.clone(),
                        interval: c.interval.clone(),
                        job: c.run_target.clone(),
                        status: c.status.clone(),
                        time,
                    }
                })
                .collect();
            Response::Crons { crons }
        }

        Query::StatusOverview => query_status::handle_status_overview(
            &state,
            &ctx.orphans,
            &ctx.metrics_health,
            ctx.start_time,
        ),

        Query::GetQueueLogs {
            queue_name,
            namespace,
            lines,
        } => query_logs::handle_get_queue_logs(queue_name, namespace, lines, &ctx.logs_path),

        Query::ListDecisions { namespace: _ } => {
            let mut decisions: Vec<DecisionSummary> = state
                .decisions
                .values()
                .filter(|d| !d.is_resolved())
                .map(|d| {
                    let job_name = state
                        .jobs
                        .get(&d.job_id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    let summary = if d.context.len() > 80 {
                        format!("{}...", &d.context[..77])
                    } else {
                        d.context.clone()
                    };
                    DecisionSummary {
                        id: d.id.to_string(),
                        job_id: d.job_id.clone(),
                        job_name,
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
                let job_name = state
                    .jobs
                    .get(&d.job_id)
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
                    job_id: d.job_id.clone(),
                    job_name,
                    agent_id: d.agent_id.clone(),
                    source: format!("{:?}", d.source).to_lowercase(),
                    context: d.context.clone(),
                    options,
                    chosen: d.chosen,
                    message: d.message.clone(),
                    created_at_ms: d.created_at_ms,
                    resolved_at_ms: d.resolved_at_ms,
                    superseded_by: d.superseded_by.as_ref().map(|id| id.to_string()),
                    namespace: d.namespace.clone(),
                })
            });
            Response::Decision { decision }
        }

        // Handled by early return above; included for exhaustiveness
        Query::ListOrphans | Query::DismissOrphan { .. } | Query::ListProjects => unreachable!(),
    }
}

/// Build a `SessionSummary` from a stored session, deriving fields from its job.
fn session_summary(s: &oj_storage::Session, state: &MaterializedState) -> SessionSummary {
    let updated_at_ms = state
        .jobs
        .get(&s.job_id)
        .and_then(|p| {
            p.step_history
                .last()
                .map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms))
        })
        .unwrap_or(0);
    let namespace = state
        .jobs
        .get(&s.job_id)
        .map(|p| p.namespace.clone())
        .unwrap_or_default();
    SessionSummary {
        id: s.id.clone(),
        namespace,
        job_id: Some(s.job_id.clone()),
        updated_at_ms,
    }
}

/// Allowed variable scope prefixes for job display.
/// Only variables with these prefixes are exposed via `oj show`.
const ALLOWED_VAR_PREFIXES: &[&str] = &[
    "var.",       // User input variables (namespaced)
    "local.",     // Computed locals from job definition
    "invoke.",    // Invocation context (e.g., invoke.dir)
    "workspace.", // Workspace context (id, root, branch, ref, nonce)
    "args.",      // Command arguments
    "item.",      // Queue item fields
];

/// Filter variables to only include user-facing scopes.
/// Variables without a declared scope prefix are excluded.
fn filter_vars_by_scope(
    vars: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    vars.iter()
        .filter(|(key, _)| {
            ALLOWED_VAR_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
#[path = "query_tests/mod.rs"]
mod tests;
