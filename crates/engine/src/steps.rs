// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job step transition effects.
//!
//! Helpers for building effects that transition jobs between steps.
//! State changes are emitted as typed events that get written to WAL
//! and applied via `apply_event()`.

use oj_core::{Effect, Event, Job, JobId, SessionId, TimerId};

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
        event: Event::JobAdvanced {
            id: JobId::new(&job.id),
            step: next_step.to_string(),
        },
    }]
}

/// Build effects to transition to failure step with error
pub fn failure_transition_effects(job: &Job, on_fail: &str, error: &str) -> Vec<Effect> {
    let job_id = JobId::new(&job.id);
    vec![
        Effect::Emit {
            event: Event::StepFailed {
                job_id: job_id.clone(),
                step: job.step.clone(),
                error: error.to_string(),
            },
        },
        Effect::Emit {
            event: Event::JobAdvanced {
                id: job_id,
                step: on_fail.to_string(),
            },
        },
    ]
}

/// Build effects to mark job as failed (terminal)
pub fn failure_effects(job: &Job, error: &str) -> Vec<Effect> {
    let job_id = JobId::new(&job.id);
    let mut effects = vec![
        Effect::CancelTimer {
            id: TimerId::liveness(&job_id),
        },
        Effect::CancelTimer {
            id: TimerId::exit_deferred(&job_id),
        },
        Effect::Emit {
            event: Event::JobAdvanced {
                id: job_id.clone(),
                step: "failed".to_string(),
            },
        },
        Effect::Emit {
            event: Event::StepFailed {
                job_id,
                step: job.step.clone(),
                error: error.to_string(),
            },
        },
    ];

    // Kill session if exists (matches completion_effects and cancellation_effects)
    if let Some(session_id) = &job.session_id {
        let session_id = SessionId::new(session_id);
        effects.push(Effect::KillSession {
            session_id: session_id.clone(),
        });
        effects.push(Effect::Emit {
            event: Event::SessionDeleted { id: session_id },
        });
    }

    effects
}

/// Build effects to transition to a cancel-cleanup step (non-terminal).
///
/// Records the cancellation of the current step, then advances to the
/// on_cancel target. The job remains non-terminal so the cleanup
/// step can execute.
pub fn cancellation_transition_effects(job: &Job, on_cancel_step: &str) -> Vec<Effect> {
    let job_id = JobId::new(&job.id);
    vec![
        Effect::Emit {
            event: Event::StepFailed {
                job_id: job_id.clone(),
                step: job.step.clone(),
                error: "cancelled".to_string(),
            },
        },
        Effect::Emit {
            event: Event::JobAdvanced {
                id: job_id,
                step: on_cancel_step.to_string(),
            },
        },
    ]
}

/// Build effects to cancel a running job.
///
/// Kills the agent (if running), kills the tmux session, cancels timers,
/// and transitions to the "cancelled" terminal state.
pub fn cancellation_effects(job: &Job) -> Vec<Effect> {
    let job_id = JobId::new(&job.id);
    let mut effects = vec![];

    // Cancel liveness and exit-deferred timers
    effects.push(Effect::CancelTimer {
        id: TimerId::liveness(&job_id),
    });
    effects.push(Effect::CancelTimer {
        id: TimerId::exit_deferred(&job_id),
    });

    // Transition to cancelled state
    if !job.is_terminal() {
        effects.push(Effect::Emit {
            event: Event::JobAdvanced {
                id: job_id.clone(),
                step: "cancelled".to_string(),
            },
        });
    }
    effects.push(Effect::Emit {
        event: Event::StepFailed {
            job_id,
            step: job.step.clone(),
            error: "cancelled".to_string(),
        },
    });

    // Kill session (covers both agent tmux sessions and shell sessions)
    if let Some(session_id) = &job.session_id {
        let session_id = SessionId::new(session_id);
        effects.push(Effect::KillSession {
            session_id: session_id.clone(),
        });
        effects.push(Effect::Emit {
            event: Event::SessionDeleted { id: session_id },
        });
    }

    effects
}

/// Build effects to complete a job
pub fn completion_effects(job: &Job) -> Vec<Effect> {
    let job_id = JobId::new(&job.id);
    let mut effects = vec![];

    // Cancel liveness and exit-deferred timers
    effects.push(Effect::CancelTimer {
        id: TimerId::liveness(&job_id),
    });
    effects.push(Effect::CancelTimer {
        id: TimerId::exit_deferred(&job_id),
    });

    // Ensure job is in done step with completed status
    if !job.is_terminal() {
        effects.push(Effect::Emit {
            event: Event::JobAdvanced {
                id: job_id.clone(),
                step: "done".to_string(),
            },
        });
    }
    effects.push(Effect::Emit {
        event: Event::StepCompleted {
            job_id,
            step: job.step.clone(),
        },
    });

    // Cleanup session if exists
    if let Some(session_id) = &job.session_id {
        let session_id = SessionId::new(session_id);
        effects.push(Effect::KillSession {
            session_id: session_id.clone(),
        });
        effects.push(Effect::Emit {
            event: Event::SessionDeleted { id: session_id },
        });
    }

    effects
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
