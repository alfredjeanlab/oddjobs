// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use oj_core::{Event, JobId, OwnerId};

use crate::protocol::{JobEntry, Response};

use super::super::{ConnectionError, ListenCtx};
use super::{emit, PruneFlags};

/// Handle a status request.
pub(crate) fn handle_status(ctx: &ListenCtx) -> Response {
    let uptime_secs = ctx.start_time.elapsed().as_secs();
    let jobs_active = {
        let state = ctx.state.lock();
        state.jobs.values().filter(|p| !p.is_terminal()).count()
    };
    let orphan_count = ctx.orphans.lock().len();

    Response::Status { uptime_secs, jobs_active, orphan_count }
}

/// Handle a job resume request.
///
/// Validates that the job exists in state or the orphan registry before
/// emitting the resume event. For orphaned jobs, emits synthetic events
/// to reconstruct the job in state, then resumes.
pub(crate) fn handle_job_resume(
    ctx: &ListenCtx,
    id: String,
    message: Option<String>,
    vars: std::collections::HashMap<String, String>,
    kill: bool,
) -> Result<Response, ConnectionError> {
    // Check if job exists in state
    let job_id = {
        let state_guard = ctx.state.lock();
        state_guard.get_job(&id).map(|p| p.id.clone())
    };

    if let Some(job_id) = job_id {
        // Auto-dismiss any pending decisions for this job
        auto_dismiss_decisions_for_job(ctx, &job_id)?;

        emit(
            &ctx.event_bus,
            Event::JobResume { id: JobId::from_string(job_id), message, vars, kill },
        )?;
        return Ok(Response::Ok);
    }

    // Not in state — check orphan registry
    let orphan = {
        let orphans_guard = ctx.orphans.lock();
        orphans_guard.iter().find(|bc| bc.job_id == id || bc.job_id.starts_with(&id)).cloned()
    };

    let Some(orphan) = orphan else {
        return Ok(Response::Error { message: format!("job not found: {}", id) });
    };

    // Orphan found — check if the runbook is available for reconstruction
    if orphan.runbook_hash.is_empty() {
        return Ok(Response::Error {
            message: format!(
                "job {} is orphaned (state lost during daemon restart) and cannot be \
                 resumed: breadcrumb missing runbook hash (written by older daemon version). \
                 Dismiss with `oj job prune --orphans` and re-run the job.",
                orphan.job_id
            ),
        });
    }

    // Verify the runbook is in state (needed for step definitions during resume)
    let runbook_available = {
        let state_guard = ctx.state.lock();
        state_guard.runbooks.contains_key(&orphan.runbook_hash)
    };

    if !runbook_available {
        return Ok(Response::Error {
            message: format!(
                "job {} is orphaned (state lost during daemon restart) and cannot be \
                 resumed: runbook is no longer available. Dismiss with \
                 `oj job prune --orphans` and re-run the job.",
                orphan.job_id
            ),
        });
    }

    // Reconstruct the job by emitting synthetic events:
    // 1. JobCreated (at current_step as initial_step)
    // 2. JobAdvanced to "failed" (so resume resets to the right step)
    // 3. JobResume (the actual resume request)
    let orphan_id = orphan.job_id.clone();
    let job_id = JobId::from_string(&orphan_id);
    let cwd = orphan.cwd.or(orphan.workspace_root).unwrap_or_default();

    emit(
        &ctx.event_bus,
        Event::JobCreated {
            id: job_id,
            kind: orphan.kind,
            name: orphan.name,
            runbook_hash: orphan.runbook_hash,
            cwd,
            vars: orphan.vars,
            initial_step: orphan.current_step,
            created_at_ms: 0,
            project: orphan.project,
            cron: None,
        },
    )?;

    emit(&ctx.event_bus, Event::JobAdvanced { id: job_id, step: "failed".to_string() })?;

    emit(&ctx.event_bus, Event::JobResume { id: job_id, message, vars, kill })?;

    // Remove from orphan registry
    {
        let mut orphans_guard = ctx.orphans.lock();
        if let Some(idx) = orphans_guard.iter().position(|bc| bc.job_id == orphan_id) {
            orphans_guard.remove(idx);
        }
    }

    Ok(Response::Ok)
}

/// Handle a bulk job resume request (--all).
///
/// Resumes all non-terminal jobs that are in a resumable state:
/// waiting, failed, or pending. With `--kill`, also resumes running jobs.
pub(crate) fn handle_job_resume_all(
    ctx: &ListenCtx,
    kill: bool,
) -> Result<Response, ConnectionError> {
    let (targets, skipped) = {
        let state_guard = ctx.state.lock();
        let mut targets: Vec<String> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();

        for job in state_guard.jobs.values() {
            // Include suspended jobs in resume_all (they're terminal but resumable)
            if job.is_terminal() && !job.is_suspended() {
                continue;
            }

            if !kill && !super::is_resumable_status(&job.step_status) {
                skipped.push((
                    job.id.clone(),
                    format!("job is {:?} (use --kill to force)", job.step_status),
                ));
                continue;
            }

            targets.push(job.id.clone());
        }

        (targets, skipped)
    };

    let mut resumed = Vec::new();
    for job_id in targets {
        // Auto-dismiss any pending decisions for this job
        auto_dismiss_decisions_for_job(ctx, &job_id)?;

        emit(
            &ctx.event_bus,
            Event::JobResume {
                id: JobId::from_string(&job_id),
                message: None,
                vars: std::collections::HashMap::new(),
                kill,
            },
        )?;
        resumed.push(job_id);
    }

    Ok(Response::JobsResumed { resumed, skipped })
}

/// Handle a job cancel request.
pub(crate) fn handle_job_cancel(
    ctx: &ListenCtx,
    ids: Vec<String>,
) -> Result<Response, ConnectionError> {
    let (acted, already_terminal, not_found) = job_action(ctx, ids, |id| Event::JobCancel { id })?;
    Ok(Response::JobsCancelled { cancelled: acted, already_terminal, not_found })
}

/// Handle a job suspend request.
pub(crate) fn handle_job_suspend(
    ctx: &ListenCtx,
    ids: Vec<String>,
) -> Result<Response, ConnectionError> {
    let (acted, already_terminal, not_found) = job_action(ctx, ids, |id| Event::JobSuspend { id })?;
    Ok(Response::JobsSuspended { suspended: acted, already_terminal, not_found })
}

/// (acted, already_terminal, not_found)
type JobActionResult = (Vec<String>, Vec<String>, Vec<String>);

/// Shared logic for cancel/suspend: iterate IDs, check terminal, emit event.
fn job_action(
    ctx: &ListenCtx,
    ids: Vec<String>,
    make_event: impl Fn(JobId) -> Event,
) -> Result<JobActionResult, ConnectionError> {
    let mut acted = Vec::new();
    let mut already_terminal = Vec::new();
    let mut not_found = Vec::new();
    for id in ids {
        match ctx.state.lock().get_job(&id).map(|p| !p.is_terminal()) {
            Some(true) => {
                emit(&ctx.event_bus, make_event(JobId::from_string(id.clone())))?;
                acted.push(id);
            }
            Some(false) => already_terminal.push(id),
            None => not_found.push(id),
        }
    }
    Ok((acted, already_terminal, not_found))
}

/// Handle job prune requests.
///
/// Removes terminal jobs (failed/cancelled/done) from state and
/// cleans up their log files. By default only prunes jobs older
/// than 12 hours; use `--all` to prune all terminal jobs.
pub(crate) fn handle_job_prune(
    ctx: &ListenCtx,
    flags: &PruneFlags<'_>,
    failed: bool,
    orphans: bool,
) -> Result<Response, ConnectionError> {
    let now_ms = super::prune_helpers::now_millis();
    let age_ms = 12 * 60 * 60 * 1000; // 12 hours

    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    // When --orphans is used alone, skip the normal terminal-job loop.
    // When combined with --all or --failed, run both.
    let prune_terminal = flags.all || failed || !orphans;

    if prune_terminal {
        let state_guard = ctx.state.lock();
        for job in state_guard.jobs.values() {
            if flags.project.is_some_and(|ns| job.project != ns) {
                continue;
            }
            if !job.is_terminal() || job.is_suspended() {
                skipped += 1;
                continue;
            }
            if failed && job.step != "failed" {
                skipped += 1;
                continue;
            }

            // Skip age check for --all, --failed on failed jobs, or cancelled jobs
            let skip_age = flags.all || (failed && job.step == "failed") || job.step == "cancelled";

            if !skip_age {
                let created = job.step_history.first().map(|r| r.started_at_ms).unwrap_or(0);
                if super::prune_helpers::within_age_threshold(false, now_ms, created, age_ms) {
                    skipped += 1;
                    continue;
                }
            }

            to_prune.push(JobEntry::from(job));
        }
    }

    if !flags.dry_run {
        for entry in &to_prune {
            emit(&ctx.event_bus, Event::JobDeleted { id: entry.id })?;
            super::prune_helpers::cleanup_job_files(&ctx.logs_path, entry.id.as_str());
        }
    }

    // When --orphans flag is set, collect orphaned jobs
    if orphans {
        let mut orphan_guard = ctx.orphans.lock();
        let drain_indices: Vec<usize> = (0..orphan_guard.len()).collect();
        for &i in drain_indices.iter().rev() {
            let bc = &orphan_guard[i];
            to_prune.push(JobEntry {
                id: JobId::from_string(&bc.job_id),
                name: bc.name.clone(),
                step: "orphaned".to_string(),
            });
            if !flags.dry_run {
                let removed = orphan_guard.remove(i);
                super::prune_helpers::cleanup_job_files(&ctx.logs_path, &removed.job_id);
            }
        }
    }

    Ok(Response::JobsPruned { pruned: to_prune, skipped })
}

/// Auto-dismiss unresolved decisions for a job owner.
///
/// When a job is resumed manually, any pending decisions for that job
/// are no longer relevant. Emits `DecisionResolved` with `chosen: None`
/// for each unresolved decision.
fn auto_dismiss_decisions_for_job(ctx: &ListenCtx, job_id: &str) -> Result<(), ConnectionError> {
    let owner: OwnerId = JobId::from_string(job_id).into();
    let unresolved: Vec<(String, String)> = {
        let state_guard = ctx.state.lock();
        state_guard
            .decisions
            .values()
            .filter(|d| d.owner == owner && !d.is_resolved())
            .map(|d| (d.id.as_str().to_string(), d.project.clone()))
            .collect()
    };

    let resolved_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    for (dec_id, project) in unresolved {
        emit(
            &ctx.event_bus,
            Event::DecisionResolved {
                id: oj_core::DecisionId::from_string(dec_id),
                choices: vec![],
                message: Some("auto-dismissed by job resume".to_string()),
                resolved_at_ms,
                project,
            },
        )?;
    }

    Ok(())
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
