// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron request handlers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::storage::MaterializedState;
use oj_core::{Event, JobId, OwnerId, RunTarget};
use parking_lot::Mutex;

use super::lifecycle::{self, Stoppable};
use super::mutations::{emit, PruneFlags};
use super::suggest;
use super::workers::hash_and_emit_runbook;
use super::{ConnectionError, ListenCtx};
use crate::protocol::{CronEntry, Response};

pub(super) struct Cron;

impl Stoppable for Cron {
    const TYPE_NAME: &'static str = "cron";
    const APPLY_STOP: bool = true;
    fn exists(state: &MaterializedState, scoped_name: &str) -> bool {
        state.crons.contains_key(scoped_name)
    }
    fn running_names(state: &MaterializedState, project: &str) -> Vec<String> {
        state
            .crons
            .values()
            .filter(|c| c.project == project && c.status == "running")
            .map(|c| c.name.clone())
            .collect()
    }
    fn stopped_event(name: &str, project: &str) -> Event {
        Event::CronStopped { cron: name.to_string(), project: project.to_string() }
    }
}

/// Validated cron definition with resolved run target.
struct ValidatedCron {
    cron_def: oj_runbook::CronDef,
    target: RunTarget,
    project_path: PathBuf,
    runbook_hash: String,
}

/// Load a cron's runbook, validate the cron definition, and return validated results.
fn load_and_validate_cron(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    cron_name: &str,
    suggest_command: &str,
) -> Result<ValidatedCron, Response> {
    let (runbook, effective_root) = super::load_runbook_with_fallback(
        project_path,
        project,
        &ctx.state,
        |root| load_runbook_for_cron(root, cron_name),
        || suggest_for_cron(Some(project_path), cron_name, project, suggest_command, &ctx.state),
    )?;
    let cron_def = runbook
        .get_cron(cron_name)
        .ok_or_else(|| Response::Error { message: format!("unknown cron: {}", cron_name) })?
        .clone();
    let target = validate_cron_run_target(&runbook, cron_name, &cron_def)?;
    let runbook_hash = hash_and_emit_runbook(&ctx.event_bus, &runbook)
        .map_err(|_| Response::Error { message: "WAL write error".to_string() })?;
    Ok(ValidatedCron { cron_def, target, project_path: effective_root, runbook_hash })
}

/// Validate cron run target references.
fn validate_cron_run_target(
    runbook: &oj_runbook::Runbook,
    cron_name: &str,
    cron_def: &oj_runbook::CronDef,
) -> Result<RunTarget, Response> {
    let target = RunTarget::from(&cron_def.run);
    match &cron_def.run {
        oj_runbook::RunDirective::Job { job } => {
            if runbook.get_job(job).is_none() {
                return Err(Response::Error {
                    message: format!("cron '{}' references unknown job '{}'", cron_name, job),
                });
            }
        }
        oj_runbook::RunDirective::Agent { agent, .. } => {
            if runbook.get_agent(agent).is_none() {
                return Err(Response::Error {
                    message: format!("cron '{}' references unknown agent '{}'", cron_name, agent),
                });
            }
        }
        oj_runbook::RunDirective::Shell(_) => {}
    }
    Ok(target)
}

/// Handle a CronStart request.
///
/// Idempotent: always emits `CronStarted`. The runtime's `handle_cron_started`
/// overwrites any existing in-memory state, so repeated starts are safe and also
/// serve to update the interval if the runbook changed.
pub(super) fn handle_cron_start(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    cron: &str,
    all: bool,
) -> Result<Response, ConnectionError> {
    if all {
        let (started, skipped) = lifecycle::handle_start_all(
            ctx,
            project_path,
            project,
            |dir| {
                oj_runbook::collect_all_crons(dir)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect()
            },
            |name| handle_cron_start(ctx, project_path, project, name, false),
            |resp| match resp {
                Response::CronStarted { cron } => Some(cron.clone()),
                _ => None,
            },
        )?;
        return Ok(Response::CronsStarted { started, skipped });
    }

    let v = match load_and_validate_cron(ctx, project_path, project, cron, "oj cron start") {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let event = Event::CronStarted {
        cron: cron.to_string(),
        project_path: v.project_path.clone(),
        runbook_hash: v.runbook_hash,
        interval: v.cron_def.interval.clone(),
        target: v.target,
        project: project.to_string(),
    };
    emit(&ctx.event_bus, event.clone())?;

    {
        let mut state = ctx.state.lock();
        state.apply_event(&event);
    }

    Ok(Response::CronStarted { cron: cron.to_string() })
}

/// Handle a CronStop request.
pub(super) fn handle_cron_stop(
    ctx: &ListenCtx,
    cron_name: &str,
    project: &str,
    project_path: Option<&Path>,
    all: bool,
) -> Result<Response, ConnectionError> {
    lifecycle::handle_stop::<Cron>(
        ctx,
        cron_name,
        project,
        all,
        || suggest_for_cron(project_path, cron_name, project, "oj cron stop", &ctx.state),
        |stopped, skipped| Response::CronsStopped { stopped, skipped },
    )
}

/// Handle a CronOnce request — run the cron's job once immediately.
pub(super) async fn handle_cron_once(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    cron: &str,
) -> Result<Response, ConnectionError> {
    let v = match load_and_validate_cron(ctx, project_path, project, cron, "oj cron once") {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let (owner, resp_name) = match &v.target {
        RunTarget::Agent(name) => {
            let crew_id = oj_core::CrewId::new();
            let resp_name = format!("agent:{}", name);
            let owner: OwnerId = crew_id.into();
            (owner, resp_name)
        }
        RunTarget::Job(job_kind) => {
            let jid = JobId::new();
            let jname = oj_runbook::job_display_name(job_kind, jid.short(8), project);
            let owner: OwnerId = jid.into();
            (owner, jname)
        }
        RunTarget::Shell(_) => {
            let jid = JobId::new();
            let jname = oj_runbook::job_display_name(cron, jid.short(8), project);
            let owner: OwnerId = jid.into();
            (owner, jname)
        }
    };

    emit(
        &ctx.event_bus,
        Event::CronOnce {
            cron: cron.to_string(),
            owner: owner.clone(),
            project_path: v.project_path,
            runbook_hash: v.runbook_hash,
            target: v.target,
            project: project.to_string(),
        },
    )?;

    match owner {
        OwnerId::Crew(crew_id) => Ok(Response::CrewStarted { crew_id, agent_name: resp_name }),
        OwnerId::Job(job_id) => Ok(Response::JobStarted { job_id, job_name: resp_name }),
    }
}

/// Handle a CronRestart request: stop (if running), reload runbook, start.
pub(super) fn handle_cron_restart(
    ctx: &ListenCtx,
    project_path: &Path,
    project: &str,
    cron_name: &str,
) -> Result<Response, ConnectionError> {
    lifecycle::handle_restart::<Cron>(ctx, project, cron_name, || {
        handle_cron_start(ctx, project_path, project, cron_name, false)
    })
}

/// Handle a CronPrune request.
pub(super) fn handle_cron_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
) -> Result<Response, ConnectionError> {
    lifecycle::handle_prune(
        ctx,
        flags,
        |s, ns| {
            let mut entries = Vec::new();
            let mut skipped = 0;
            for c in s.crons.values() {
                if ns.is_some_and(|n| c.project != n) {
                    continue;
                }
                if c.status != "stopped" {
                    skipped += 1;
                    continue;
                }
                entries.push(CronEntry::from(c));
            }
            (entries, skipped)
        },
        |e| Event::CronDeleted { cron: e.name.clone(), project: e.project.clone() },
        |pruned, skipped| Response::CronsPruned { pruned, skipped },
    )
}

fn suggest_for_cron(
    root: Option<&Path>,
    name: &str,
    ns: &str,
    cmd: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> String {
    let ns_owned = ns.to_string();
    let root = root.map(|r| r.to_path_buf());
    suggest::suggest_for_resource(
        name,
        ns,
        cmd,
        state,
        suggest::ResourceType::Cron,
        || {
            root.map(|r| {
                oj_runbook::collect_all_crons(&r.join(".oj/runbooks"))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect()
            })
            .unwrap_or_default()
        },
        move |s| {
            s.crons.values().filter(|c| c.project == ns_owned).map(|c| c.name.clone()).collect()
        },
    )
}

fn load_runbook_for_cron(
    project_path: &Path,
    cron_name: &str,
) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_path.join(".oj/runbooks");
    oj_runbook::find_runbook_by_cron(&runbook_dir, cron_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no runbook found containing cron '{}'", cron_name))
}

#[cfg(test)]
#[path = "crons_tests.rs"]
mod tests;
