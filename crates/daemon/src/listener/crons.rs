// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron request handlers.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{Event, IdGen, JobId, UuidIdGen};
use oj_storage::MaterializedState;

use crate::protocol::Response;

use super::mutations::emit;
use super::suggest;
use super::workers::hash_and_emit_runbook;
use super::ConnectionError;
use super::ListenCtx;

/// Handle a CronStart request.
///
/// Idempotent: always emits `CronStarted`. The runtime's `handle_cron_started`
/// overwrites any existing in-memory state, so repeated starts are safe and also
/// serve to update the interval if the runbook changed.
pub(super) fn handle_cron_start(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    cron_name: &str,
    all: bool,
) -> Result<Response, ConnectionError> {
    if all {
        return handle_cron_start_all(ctx, project_root, namespace);
    }

    // Load runbook to validate cron exists.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        &ctx.state,
        |root| load_runbook_for_cron(root, cron_name),
        || {
            suggest_for_cron(
                Some(project_root),
                cron_name,
                namespace,
                "oj cron start",
                &ctx.state,
            )
        },
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Validate cron definition exists
    let cron_def = match runbook.get_cron(cron_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown cron: {}", cron_name),
            })
        }
    };

    // Validate run target references
    let run_target = match &cron_def.run {
        oj_runbook::RunDirective::Job { job } => {
            if runbook.get_job(job).is_none() {
                return Ok(Response::Error {
                    message: format!("cron '{}' references unknown job '{}'", cron_name, job),
                });
            }
            format!("job:{}", job)
        }
        oj_runbook::RunDirective::Agent { agent, .. } => {
            if runbook.get_agent(agent).is_none() {
                return Ok(Response::Error {
                    message: format!("cron '{}' references unknown agent '{}'", cron_name, agent),
                });
            }
            format!("agent:{}", agent)
        }
        oj_runbook::RunDirective::Shell(cmd) => {
            format!("shell:{}", cmd)
        }
    };

    // Hash runbook and emit RunbookLoaded for WAL persistence
    let runbook_hash = hash_and_emit_runbook(&ctx.event_bus, &runbook)?;

    // Emit CronStarted event
    let event = Event::CronStarted {
        cron_name: cron_name.to_string(),
        project_root: project_root.to_path_buf(),
        runbook_hash,
        interval: cron_def.interval.clone(),
        run_target,
        namespace: namespace.to_string(),
    };

    emit(&ctx.event_bus, event.clone())?;

    // Apply to materialized state before responding so queries see it
    // immediately. apply_event is idempotent so the second apply when the
    // main loop processes this event from the WAL is harmless.
    {
        let mut state = ctx.state.lock();
        state.apply_event(&event);
    }

    Ok(Response::CronStarted {
        cron_name: cron_name.to_string(),
    })
}

/// Handle starting all crons defined in runbooks.
fn handle_cron_start_all(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
) -> Result<Response, ConnectionError> {
    let runbook_dir = project_root.join(".oj/runbooks");
    let names = oj_runbook::collect_all_crons(&runbook_dir)
        .unwrap_or_default()
        .into_iter()
        .map(|(name, _)| name);

    let (started, skipped) = super::collect_start_results(
        names,
        |name| handle_cron_start(ctx, project_root, namespace, name, false),
        |resp| match resp {
            Response::CronStarted { cron_name } => Some(cron_name.clone()),
            _ => None,
        },
    )?;

    Ok(Response::CronsStarted { started, skipped })
}

/// Handle a CronStop request.
pub(super) fn handle_cron_stop(
    ctx: &ListenCtx,
    cron_name: &str,
    namespace: &str,
    project_root: Option<&Path>,
    all: bool,
) -> Result<Response, ConnectionError> {
    if all {
        return handle_cron_stop_all(ctx, namespace);
    }

    // Check if cron exists in state
    if let Err(resp) = super::require_scoped_resource(
        &ctx.state,
        namespace,
        cron_name,
        "cron",
        |s, k| s.crons.contains_key(k),
        || {
            suggest_for_cron(
                project_root,
                cron_name,
                namespace,
                "oj cron stop",
                &ctx.state,
            )
        },
    ) {
        return Ok(resp);
    }

    let event = Event::CronStopped {
        cron_name: cron_name.to_string(),
        namespace: namespace.to_string(),
    };

    emit(&ctx.event_bus, event.clone())?;

    // Apply to materialized state before responding so queries see it
    // immediately. apply_event is idempotent.
    {
        let mut state = ctx.state.lock();
        state.apply_event(&event);
    }

    Ok(Response::Ok)
}

/// Handle stopping all running crons in a namespace.
fn handle_cron_stop_all(ctx: &ListenCtx, namespace: &str) -> Result<Response, ConnectionError> {
    let running_crons: Vec<String> = {
        let state = ctx.state.lock();
        state
            .crons
            .values()
            .filter(|c| c.namespace == namespace && c.status == "running")
            .map(|c| c.name.clone())
            .collect()
    };

    let mut stopped = Vec::new();
    let mut skipped = Vec::new();

    for name in running_crons {
        let event = Event::CronStopped {
            cron_name: name.clone(),
            namespace: namespace.to_string(),
        };
        match emit(&ctx.event_bus, event.clone()) {
            Ok(()) => {
                let mut state = ctx.state.lock();
                state.apply_event(&event);
                stopped.push(name);
            }
            Err(e) => skipped.push((name, e.to_string())),
        }
    }

    Ok(Response::CronsStopped { stopped, skipped })
}

/// Handle a CronOnce request — run the cron's job once immediately.
pub(super) async fn handle_cron_once(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    cron_name: &str,
) -> Result<Response, ConnectionError> {
    // Load runbook to validate cron exists.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        &ctx.state,
        |root| load_runbook_for_cron(root, cron_name),
        || {
            suggest_for_cron(
                Some(project_root),
                cron_name,
                namespace,
                "oj cron once",
                &ctx.state,
            )
        },
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Validate cron definition exists
    let cron_def = match runbook.get_cron(cron_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown cron: {}", cron_name),
            })
        }
    };

    // Validate run target references and build event
    let (job_kind, run_target) = match &cron_def.run {
        oj_runbook::RunDirective::Job { job } => {
            if runbook.get_job(job).is_none() {
                return Ok(Response::Error {
                    message: format!("cron '{}' references unknown job '{}'", cron_name, job),
                });
            }
            (job.clone(), format!("job:{}", job))
        }
        oj_runbook::RunDirective::Agent { agent, .. } => {
            if runbook.get_agent(agent).is_none() {
                return Ok(Response::Error {
                    message: format!("cron '{}' references unknown agent '{}'", cron_name, agent),
                });
            }
            (String::new(), format!("agent:{}", agent))
        }
        oj_runbook::RunDirective::Shell(cmd) => (cron_name.to_string(), format!("shell:{}", cmd)),
    };

    // Hash runbook and emit RunbookLoaded for WAL persistence
    let runbook_hash = hash_and_emit_runbook(&ctx.event_bus, &runbook)?;

    let is_agent = run_target.starts_with("agent:");

    if is_agent {
        let agent_name = run_target.strip_prefix("agent:").unwrap_or("").to_string();
        let agent_run_id = UuidIdGen.next();

        let event = Event::CronOnce {
            cron_name: cron_name.to_string(),
            job_id: JobId::new(""),
            job_name: String::new(),
            job_kind: String::new(),
            agent_run_id: Some(agent_run_id.clone()),
            agent_name: Some(agent_name.clone()),
            project_root: project_root.to_path_buf(),
            runbook_hash: runbook_hash.clone(),
            run_target,
            namespace: namespace.to_string(),
        };

        emit(&ctx.event_bus, event)?;

        Ok(Response::CommandStarted {
            job_id: agent_run_id,
            job_name: format!("agent:{}", agent_name),
        })
    } else {
        // Generate job ID
        let job_id = JobId::new(UuidIdGen.next());
        let job_display_name = oj_runbook::job_display_name(&job_kind, job_id.short(8), namespace);

        // Emit CronOnce event to create job via the cron code path
        let event = Event::CronOnce {
            cron_name: cron_name.to_string(),
            job_id: job_id.clone(),
            job_name: job_display_name.clone(),
            job_kind: job_kind.clone(),
            agent_run_id: None,
            agent_name: None,
            project_root: project_root.to_path_buf(),
            runbook_hash: runbook_hash.clone(),
            run_target,
            namespace: namespace.to_string(),
        };

        emit(&ctx.event_bus, event)?;

        Ok(Response::CommandStarted {
            job_id: job_id.to_string(),
            job_name: job_display_name,
        })
    }
}

/// Handle a CronRestart request: stop (if running), reload runbook, start.
pub(super) fn handle_cron_restart(
    ctx: &ListenCtx,
    project_root: &Path,
    namespace: &str,
    cron_name: &str,
) -> Result<Response, ConnectionError> {
    // Stop cron if it exists in state
    if super::scoped_exists(&ctx.state, namespace, cron_name, |s, k| {
        s.crons.contains_key(k)
    }) {
        emit(
            &ctx.event_bus,
            Event::CronStopped {
                cron_name: cron_name.to_string(),
                namespace: namespace.to_string(),
            },
        )?;
    }

    // Start with fresh runbook
    handle_cron_start(ctx, project_root, namespace, cron_name, false)
}

#[cfg(test)]
#[path = "crons_tests.rs"]
mod tests;

/// Load a runbook that contains the given cron name.
fn load_runbook_for_cron(
    project_root: &Path,
    cron_name: &str,
) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_root.join(".oj/runbooks");
    oj_runbook::find_runbook_by_cron(&runbook_dir, cron_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no runbook found containing cron '{}'", cron_name))
}

/// Generate a "did you mean" suggestion for a cron name.
fn suggest_for_cron(
    project_root: Option<&Path>,
    cron_name: &str,
    namespace: &str,
    command_prefix: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> String {
    let ns = namespace.to_string();
    let root = project_root.map(|r| r.to_path_buf());
    suggest::suggest_for_resource(
        cron_name,
        namespace,
        command_prefix,
        state,
        suggest::ResourceType::Cron,
        || {
            root.map(|r| {
                oj_runbook::collect_all_crons(&r.join(".oj/runbooks"))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect()
            })
            .unwrap_or_default()
        },
        |state| {
            state
                .crons
                .values()
                .filter(|c| c.namespace == ns)
                .map(|c| c.name.clone())
                .collect()
        },
    )
}
