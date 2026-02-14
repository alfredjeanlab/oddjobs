// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job step transition effects.
//!
//! Helpers for building effects that transition jobs between steps.
//! State changes are emitted as typed events that get written to WAL
//! and applied via `apply_event()`.

use oj_core::{AgentId, Effect, Event, Job, JobId, TimerId};

/// Build effects to mark a step as running
pub fn step_start_effects(job_id: &JobId, step: &str) -> Vec<Effect> {
    vec![Effect::Emit {
        event: Event::StepStarted {
            job_id: job_id.clone(),
            step: step.to_string(),
            agent_id: None,
            agent_name: None,
        },
    }]
}

/// Build effects to transition to the next step
pub fn step_transition_effects(job: &Job, next_step: &str) -> Vec<Effect> {
    vec![Effect::Emit {
        event: Event::JobAdvanced { id: JobId::from_string(&job.id), step: next_step.to_string() },
    }]
}

/// Build effects to transition to failure step with error
pub fn failure_transition_effects(job: &Job, on_fail: &str, error: &str) -> Vec<Effect> {
    let job_id = JobId::from_string(&job.id);
    vec![
        Effect::Emit {
            event: Event::StepFailed {
                job_id: job_id.clone(),
                step: job.step.clone(),
                error: error.to_string(),
            },
        },
        Effect::Emit { event: Event::JobAdvanced { id: job_id, step: on_fail.to_string() } },
    ]
}

/// Build effects after on_fail cleanup step completes: mark the cleanup step
/// as completed, then transition to the "failed" terminal state.
pub fn failure_after_cleanup_effects(job: &Job, error: &str) -> Vec<Effect> {
    let job_id = JobId::from_string(&job.id);
    let mut effects = vec![
        Effect::CancelTimer { id: TimerId::liveness(&job_id) },
        Effect::CancelTimer { id: TimerId::exit_deferred(&job_id) },
        Effect::Emit {
            event: Event::StepCompleted { job_id: job_id.clone(), step: job.step.clone() },
        },
        Effect::Emit {
            event: Event::JobAdvanced { id: job_id.clone(), step: "failed".to_string() },
        },
        Effect::Emit {
            event: Event::StepFailed {
                job_id,
                step: "failed".to_string(),
                error: error.to_string(),
            },
        },
    ];

    // Kill agent if the cleanup step was an agent step
    if let Some(agent_id) =
        job.step_history.iter().rfind(|r| r.name == job.step).and_then(|r| r.agent_id.as_ref())
    {
        effects.push(Effect::KillAgent { agent_id: AgentId::from_string(agent_id) });
    }

    effects
}

/// Build effects to mark job as failed (terminal).
///
/// Always emits JobAdvanced (even if already terminal) because failure_effects
/// is called from circuit breaker paths that need the advance unconditionally.
pub fn failure_effects(job: &Job, error: &str) -> Vec<Effect> {
    let job_id = JobId::from_string(&job.id);
    terminal_job_effects(
        job,
        "failed",
        true,
        Effect::Emit {
            event: Event::StepFailed { job_id, step: job.step.clone(), error: error.to_string() },
        },
    )
}

/// Build effects to transition to a cancel-cleanup step (non-terminal).
///
/// Records the cancellation of the current step, then advances to the
/// on_cancel target. The job remains non-terminal so the cleanup
/// step can execute.
pub fn cancellation_transition_effects(job: &Job, on_cancel_step: &str) -> Vec<Effect> {
    let job_id = JobId::from_string(&job.id);
    vec![
        Effect::Emit {
            event: Event::StepFailed {
                job_id: job_id.clone(),
                step: job.step.clone(),
                error: "cancelled".to_string(),
            },
        },
        Effect::Emit { event: Event::JobAdvanced { id: job_id, step: on_cancel_step.to_string() } },
    ]
}

/// Build effects to cancel a running job (terminal).
pub fn cancellation_effects(job: &Job) -> Vec<Effect> {
    let job_id = JobId::from_string(&job.id);
    terminal_job_effects(
        job,
        "cancelled",
        false,
        Effect::Emit {
            event: Event::StepFailed {
                job_id,
                step: job.step.clone(),
                error: "cancelled".to_string(),
            },
        },
    )
}

/// Build effects to suspend a running job (terminal).
///
/// Unlike cancellation, does NOT delete the workspace — preserves everything for resume.
pub fn suspension_effects(job: &Job) -> Vec<Effect> {
    let job_id = JobId::from_string(&job.id);
    terminal_job_effects(
        job,
        "suspended",
        false,
        Effect::Emit {
            event: Event::StepFailed {
                job_id,
                step: job.step.clone(),
                error: "suspended".to_string(),
            },
        },
    )
}

/// Build effects to complete a job (terminal).
pub fn completion_effects(job: &Job) -> Vec<Effect> {
    let job_id = JobId::from_string(&job.id);
    terminal_job_effects(
        job,
        "done",
        false,
        Effect::Emit { event: Event::StepCompleted { job_id, step: job.step.clone() } },
    )
}

/// Shared skeleton for terminal job effects: cancel timers, advance to terminal
/// step (if not already there), emit the step outcome, and kill the session.
fn terminal_job_effects(
    job: &Job,
    terminal_step: &str,
    force_advance: bool,
    outcome: Effect,
) -> Vec<Effect> {
    let job_id = JobId::from_string(&job.id);
    let mut effects = vec![
        Effect::CancelTimer { id: TimerId::liveness(&job_id) },
        Effect::CancelTimer { id: TimerId::exit_deferred(&job_id) },
    ];

    if force_advance || !job.is_terminal() {
        effects.push(Effect::Emit {
            event: Event::JobAdvanced { id: job_id, step: terminal_step.to_string() },
        });
    }
    effects.push(outcome);

    // Kill agent via AgentAdapter (handles session cleanup internally)
    if let Some(agent_id) =
        job.step_history.iter().rfind(|r| r.name == job.step).and_then(|r| r.agent_id.as_ref())
    {
        effects.push(Effect::KillAgent { agent_id: AgentId::from_string(agent_id) });
    }

    effects
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
