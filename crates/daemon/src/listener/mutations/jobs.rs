// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use oj_core::{Event, JobId, OwnerId};

use crate::protocol::{JobEntry, Response};

use super::super::{ConnectionError, ListenCtx};
use super::{emit, PruneFlags};

/// Handle a status request.
pub(crate) fn handle_status(ctx: &ListenCtx) -> Response {
    let uptime_secs = ctx.start_time.elapsed().as_secs();
    let (jobs_active, sessions_active) = {
        let state = ctx.state.lock();
        let active = state.jobs.values().filter(|p| !p.is_terminal()).count();
        let sessions = state.sessions.len();
        (active, sessions)
    };
    let orphan_count = ctx.orphans.lock().len();

    Response::Status {
        uptime_secs,
        jobs_active,
        sessions_active,
        orphan_count,
    }
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
            Event::JobResume {
                id: JobId::new(job_id),
                message,
                vars,
                kill,
            },
        )?;
        return Ok(Response::Ok);
    }

    // Not in state — check orphan registry
    let orphan = {
        let orphans_guard = ctx.orphans.lock();
        orphans_guard
            .iter()
            .find(|bc| bc.job_id == id || bc.job_id.starts_with(&id))
            .cloned()
    };

    let Some(orphan) = orphan else {
        return Ok(Response::Error {
            message: format!("job not found: {}", id),
        });
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
    let job_id = JobId::new(&orphan_id);
    let cwd = orphan.cwd.or(orphan.workspace_root).unwrap_or_default();

    emit(
        &ctx.event_bus,
        Event::JobCreated {
            id: job_id.clone(),
            kind: orphan.kind,
            name: orphan.name,
            runbook_hash: orphan.runbook_hash,
            cwd,
            vars: orphan.vars,
            initial_step: orphan.current_step,
            created_at_epoch_ms: 0,
            namespace: orphan.project,
            cron_name: None,
        },
    )?;

    emit(
        &ctx.event_bus,
        Event::JobAdvanced {
            id: job_id.clone(),
            step: "failed".to_string(),
        },
    )?;

    emit(
        &ctx.event_bus,
        Event::JobResume {
            id: job_id,
            message,
            vars,
            kill,
        },
    )?;

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

            if !kill {
                // Without --kill, only resume jobs in a resumable state
                if !job.step_status.is_waiting()
                    && !matches!(
                        job.step_status,
                        oj_core::StepStatus::Failed
                            | oj_core::StepStatus::Pending
                            | oj_core::StepStatus::Suspended
                    )
                {
                    skipped.push((
                        job.id.clone(),
                        format!("job is {:?} (use --kill to force)", job.step_status),
                    ));
                    continue;
                }
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
                id: JobId::new(&job_id),
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
    let mut cancelled = Vec::new();
    let mut already_terminal = Vec::new();
    let mut not_found = Vec::new();

    for id in ids {
        let is_valid = {
            let state_guard = ctx.state.lock();
            state_guard.get_job(&id).map(|p| !p.is_terminal())
        };

        match is_valid {
            Some(true) => {
                emit(
                    &ctx.event_bus,
                    Event::JobCancel {
                        id: JobId::new(id.clone()),
                    },
                )?;
                cancelled.push(id);
            }
            Some(false) => {
                already_terminal.push(id);
            }
            None => {
                not_found.push(id);
            }
        }
    }

    Ok(Response::JobsCancelled {
        cancelled,
        already_terminal,
        not_found,
    })
}

/// Handle a job suspend request.
pub(crate) fn handle_job_suspend(
    ctx: &ListenCtx,
    ids: Vec<String>,
) -> Result<Response, ConnectionError> {
    let mut suspended = Vec::new();
    let mut already_terminal = Vec::new();
    let mut not_found = Vec::new();

    for id in ids {
        let is_valid = {
            let state_guard = ctx.state.lock();
            state_guard.get_job(&id).map(|p| !p.is_terminal())
        };

        match is_valid {
            Some(true) => {
                emit(
                    &ctx.event_bus,
                    Event::JobSuspend {
                        id: JobId::new(id.clone()),
                    },
                )?;
                suspended.push(id);
            }
            Some(false) => {
                already_terminal.push(id);
            }
            None => {
                not_found.push(id);
            }
        }
    }

    Ok(Response::JobsSuspended {
        suspended,
        already_terminal,
        not_found,
    })
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
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_threshold_ms = 12 * 60 * 60 * 1000; // 12 hours in ms

    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    // When --orphans is used alone, skip the normal terminal-job loop.
    // When combined with --all or --failed, run both.
    let prune_terminal = flags.all || failed || !orphans;

    if prune_terminal {
        let state_guard = ctx.state.lock();
        for job in state_guard.jobs.values() {
            // Filter by namespace when --project is specified
            if let Some(ns) = flags.namespace {
                if job.namespace != ns {
                    continue;
                }
            }

            if !job.is_terminal() {
                skipped += 1;
                continue;
            }

            // Never prune suspended jobs — they are preserved for later resume
            if job.is_suspended() {
                skipped += 1;
                continue;
            }

            // --failed flag: only prune failed jobs (skip done/cancelled)
            if failed && job.step != "failed" {
                skipped += 1;
                continue;
            }

            // Determine if this job skips the age check:
            // - --all: everything skips age check
            // - --failed: failed jobs skip age check
            // - cancelled jobs always skip age check (default behavior)
            let skip_age_check =
                flags.all || (failed && job.step == "failed") || job.step == "cancelled";

            if !skip_age_check {
                let created_at_ms = job
                    .step_history
                    .first()
                    .map(|r| r.started_at_ms)
                    .unwrap_or(0);
                if created_at_ms > 0 && now_ms.saturating_sub(created_at_ms) < age_threshold_ms {
                    skipped += 1;
                    continue;
                }
            }

            to_prune.push(JobEntry {
                id: job.id.clone(),
                name: job.name.clone(),
                step: job.step.clone(),
            });
        }
    }

    if !flags.dry_run {
        for entry in &to_prune {
            emit(
                &ctx.event_bus,
                Event::JobDeleted {
                    id: JobId::new(entry.id.clone()),
                },
            )?;
            super::prune_helpers::cleanup_job_files(&ctx.logs_path, &entry.id);
        }
    }

    // When --orphans flag is set, collect orphaned jobs
    if orphans {
        let mut orphan_guard = ctx.orphans.lock();
        let drain_indices: Vec<usize> = (0..orphan_guard.len()).collect();
        for &i in drain_indices.iter().rev() {
            let bc = &orphan_guard[i];
            to_prune.push(JobEntry {
                id: bc.job_id.clone(),
                name: bc.name.clone(),
                step: "orphaned".to_string(),
            });
            if !flags.dry_run {
                let removed = orphan_guard.remove(i);
                super::prune_helpers::cleanup_job_files(&ctx.logs_path, &removed.job_id);
            }
        }
    }

    Ok(Response::JobsPruned {
        pruned: to_prune,
        skipped,
    })
}

/// Auto-dismiss unresolved decisions for a job owner.
///
/// When a job is resumed manually, any pending decisions for that job
/// are no longer relevant. Emits `DecisionResolved` with `chosen: None`
/// for each unresolved decision.
fn auto_dismiss_decisions_for_job(ctx: &ListenCtx, job_id: &str) -> Result<(), ConnectionError> {
    let owner = OwnerId::Job(JobId::new(job_id));
    let unresolved: Vec<(String, String)> = {
        let state_guard = ctx.state.lock();
        state_guard
            .decisions
            .values()
            .filter(|d| d.owner == owner && !d.is_resolved())
            .map(|d| (d.id.as_str().to_string(), d.namespace.clone()))
            .collect()
    };

    let resolved_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    for (dec_id, namespace) in unresolved {
        emit(
            &ctx.event_bus,
            Event::DecisionResolved {
                id: dec_id,
                chosen: None,
                choices: vec![],
                message: Some("auto-dismissed by job resume".to_string()),
                resolved_at_ms,
                namespace,
            },
        )?;
    }

    Ok(())
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
