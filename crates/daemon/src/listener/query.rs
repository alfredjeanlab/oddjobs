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

mod helpers {
    use oj_core::OwnerId;
    use oj_storage::MaterializedState;

    /// Get a display name for an owner by looking up the job or crew name.
    pub(super) fn owner_display_name(owner: &OwnerId, state: &MaterializedState) -> String {
        match owner {
            OwnerId::Job(job_id) => {
                state.jobs.get(job_id.as_str()).map(|p| p.name.clone()).unwrap_or_default()
            }
            OwnerId::Crew(crew_id) => {
                state.crew.get(crew_id.as_str()).map(|r| r.command_name.clone()).unwrap_or_default()
            }
        }
    }
}

use crate::protocol::{
    CronSummary, DecisionDetail, DecisionSummary, JobDetail, JobSummary, Query, QueueItemSummary,
    Response, StepRecordDetail, WorkerSummary, WorkspaceDetail, WorkspaceSummary,
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
            let mut jobs: Vec<JobSummary> = state.jobs.values().map(JobSummary::from).collect();

            query_orphans::append_orphan_summaries(&mut jobs, &ctx.orphans);

            Response::Jobs { jobs }
        }

        Query::GetJob { id } => {
            let job = state.get_job(&id).map(|p| {
                let steps: Vec<StepRecordDetail> =
                    p.step_history.iter().map(StepRecordDetail::from).collect();

                // Compute agent summaries from log files
                let project = namespace_to_option(&p.project);
                let agents =
                    query_agents::compute_agent_summaries(&p.id, &steps, &ctx.logs_path, project);

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
                    error: p.error.clone(),
                    steps,
                    agents,
                    project: p.project.clone(),
                })
            });

            // If not found in state, check orphans
            let job = job.or_else(|| query_orphans::find_orphan_detail(&ctx.orphans, &id));

            Response::Job { job }
        }

        Query::GetAgent { agent_id } => {
            query_agents::handle_get_agent(agent_id, &state, &ctx.logs_path)
        }

        Query::ListWorkspaces => {
            let workspaces = state
                .workspaces
                .values()
                .map(|w| {
                    let project = match &w.owner {
                        oj_core::OwnerId::Job(job_id) => state
                            .jobs
                            .get(job_id.as_str())
                            .map(|p| p.project.clone())
                            .unwrap_or_default(),
                        oj_core::OwnerId::Crew(run_id) => state
                            .crew
                            .get(run_id.as_str())
                            .map(|run| run.project.clone())
                            .unwrap_or_default(),
                    };
                    WorkspaceSummary {
                        id: w.id.clone(),
                        path: w.path.clone(),
                        branch: w.branch.clone(),
                        status: w.status.to_string(),
                        created_at_ms: w.created_at_ms,
                        project,
                    }
                })
                .collect();
            Response::Workspaces { workspaces }
        }

        Query::GetWorkspace { id } => {
            let workspace = state.workspaces.get(&id).map(|w| Box::new(WorkspaceDetail::from(w)));
            Response::Workspace { workspace }
        }

        Query::GetAgentLogs { id, step, lines, offset } => {
            query_logs::handle_get_agent_logs(id, step, lines, offset, &state, &ctx.logs_path)
        }

        Query::GetJobLogs { id, lines, offset } => {
            query_logs::handle_get_job_logs(id, lines, offset, &state, &ctx.orphans, &ctx.logs_path)
        }

        Query::ListQueues { project_path, project } => {
            query_queues::list_queues(&state, &project_path, &project)
        }

        Query::ListQueueItems { queue, project, project_path } => {
            let key = scoped_name(&project, &queue);

            match state.queue_items.get(&key) {
                Some(queue_items) => {
                    let items = queue_items.iter().map(QueueItemSummary::from).collect();
                    Response::QueueItems { items }
                }
                None => {
                    // Queue not in state — check if it exists in runbooks
                    let in_runbook = project_path.as_ref().is_some_and(|root| {
                        oj_runbook::find_runbook_by_queue(&root.join(".oj/runbooks"), &queue)
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
                                if ns == project {
                                    Some(name.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if let Some(ref root) = project_path {
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
                            &queue,
                            &project,
                            "oj queue show",
                            &state,
                            suggest::ResourceType::Queue,
                            &candidates,
                        );

                        if hint.is_empty() {
                            Response::QueueItems { items: vec![] }
                        } else {
                            Response::Error { message: format!("unknown queue: {}{}", queue, hint) }
                        }
                    }
                }
            }
        }

        Query::ListAgents { job_id, status } => {
            query_agents::handle_list_agents(job_id, status, &state, &ctx.logs_path)
        }

        Query::GetWorkerLogs { name, project, lines, project_path, offset } => {
            query_logs::handle_get_worker_logs(
                name,
                project,
                lines,
                offset,
                project_path,
                &state,
                &ctx.logs_path,
            )
        }

        Query::ListWorkers => {
            let workers = state
                .workers
                .values()
                .map(|w| {
                    let updated_at_ms = worker_updated_at_ms(w, &state);
                    WorkerSummary::from_worker(w, updated_at_ms)
                })
                .collect();
            Response::Workers { workers }
        }

        Query::GetCronLogs { name, project, lines, project_path, offset } => {
            query_logs::handle_get_cron_logs(
                name,
                project,
                lines,
                offset,
                project_path,
                &state,
                &ctx.logs_path,
            )
        }

        Query::ListCrons => {
            let now_ms =
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
            let crons = state
                .crons
                .values()
                .map(|c| {
                    let time = query_crons::cron_time_display(c, now_ms);
                    CronSummary::from_cron(c, time)
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

        Query::GetQueueLogs { queue, project, lines, offset } => {
            query_logs::handle_get_queue_logs(queue, project, lines, offset, &ctx.logs_path)
        }

        Query::ListDecisions { project: _ } => {
            let mut decisions: Vec<DecisionSummary> = state
                .decisions
                .values()
                .filter(|d| !d.is_resolved())
                .map(|d| {
                    let owner_name = helpers::owner_display_name(&d.owner, &state);
                    DecisionSummary::from_decision(d, owner_name)
                })
                .collect();
            decisions.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
            Response::Decisions { decisions }
        }

        Query::GetDecision { id } => {
            let decision = state.get_decision(&id).map(|d| {
                let owner_name = helpers::owner_display_name(&d.owner, &state);
                Box::new(DecisionDetail::from_decision(d, owner_name))
            });
            Response::Decision { decision }
        }

        // Handled by early return above; included for exhaustiveness
        Query::ListOrphans | Query::DismissOrphan { .. } | Query::ListProjects => unreachable!(),
    }
}

/// Derive `updated_at_ms` for a worker from its most recently active job.
pub(super) fn worker_updated_at_ms(w: &oj_storage::WorkerRecord, state: &MaterializedState) -> u64 {
    w.active
        .iter()
        .filter_map(|pid| state.jobs.get(pid))
        .filter_map(|p| p.step_history.last().map(|r| r.finished_at_ms.unwrap_or(r.started_at_ms)))
        .max()
        .unwrap_or(0)
}

/// Allowed variable scope prefixes for job display.
/// Only variables with these prefixes are exposed via `oj show`.
const ALLOWED_VAR_PREFIXES: &[&str] = &[
    "var.",    // User input variables (namespaced)
    "local.",  // Computed locals from job definition
    "invoke.", // Invocation context (e.g., invoke.dir)
    "source.", // Source context (id, root, branch, ref, nonce)
    "args.",   // Command arguments
];

/// Filter variables to only include user-facing scopes.
/// Variables without a declared scope prefix are excluded.
fn filter_vars_by_scope(
    vars: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    vars.iter()
        .filter(|(key, _)| ALLOWED_VAR_PREFIXES.iter().any(|prefix| key.starts_with(prefix)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
#[path = "query_tests/mod.rs"]
mod tests;
