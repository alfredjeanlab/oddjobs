// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Pipeline step transition effects.
//!
//! Helpers for building effects that transition pipelines between steps.
//! State changes are emitted as typed events that get written to WAL
//! and applied via `apply_event()`.

use oj_core::{Effect, Event, Pipeline, PipelineId, SessionId, TimerId};

/// Build effects to mark a step as running
pub fn step_start_effects(pipeline_id: &PipelineId, step: &str) -> Vec<Effect> {
    vec![Effect::Emit {
        event: Event::StepStarted {
            pipeline_id: pipeline_id.clone(),
            step: step.to_string(),
            agent_id: None,
            agent_name: None,
        },
    }]
}

/// Build effects to transition to the next step
pub fn step_transition_effects(pipeline: &Pipeline, next_step: &str) -> Vec<Effect> {
    vec![Effect::Emit {
        event: Event::PipelineAdvanced {
            id: PipelineId::new(&pipeline.id),
            step: next_step.to_string(),
        },
    }]
}

/// Build effects to transition to failure step with error
pub fn failure_transition_effects(pipeline: &Pipeline, on_fail: &str, error: &str) -> Vec<Effect> {
    let pipeline_id = PipelineId::new(&pipeline.id);
    vec![
        Effect::Emit {
            event: Event::StepFailed {
                pipeline_id: pipeline_id.clone(),
                step: pipeline.step.clone(),
                error: error.to_string(),
            },
        },
        Effect::Emit {
            event: Event::PipelineAdvanced {
                id: pipeline_id,
                step: on_fail.to_string(),
            },
        },
    ]
}

/// Build effects to mark pipeline as failed (terminal)
pub fn failure_effects(pipeline: &Pipeline, error: &str) -> Vec<Effect> {
    let pipeline_id = PipelineId::new(&pipeline.id);
    let mut effects = vec![
        Effect::CancelTimer {
            id: TimerId::liveness(&pipeline_id),
        },
        Effect::CancelTimer {
            id: TimerId::exit_deferred(&pipeline_id),
        },
        Effect::Emit {
            event: Event::PipelineAdvanced {
                id: pipeline_id.clone(),
                step: "failed".to_string(),
            },
        },
        Effect::Emit {
            event: Event::StepFailed {
                pipeline_id,
                step: pipeline.step.clone(),
                error: error.to_string(),
            },
        },
    ];

    // Kill session if exists (matches completion_effects and cancellation_effects)
    if let Some(session_id) = &pipeline.session_id {
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
/// on_cancel target. The pipeline remains non-terminal so the cleanup
/// step can execute.
pub fn cancellation_transition_effects(pipeline: &Pipeline, on_cancel_step: &str) -> Vec<Effect> {
    let pipeline_id = PipelineId::new(&pipeline.id);
    vec![
        Effect::Emit {
            event: Event::StepFailed {
                pipeline_id: pipeline_id.clone(),
                step: pipeline.step.clone(),
                error: "cancelled".to_string(),
            },
        },
        Effect::Emit {
            event: Event::PipelineAdvanced {
                id: pipeline_id,
                step: on_cancel_step.to_string(),
            },
        },
    ]
}

/// Build effects to cancel a running pipeline.
///
/// Kills the agent (if running), kills the tmux session, cancels timers,
/// and transitions to the "cancelled" terminal state.
pub fn cancellation_effects(pipeline: &Pipeline) -> Vec<Effect> {
    let pipeline_id = PipelineId::new(&pipeline.id);
    let mut effects = vec![];

    // Cancel liveness and exit-deferred timers
    effects.push(Effect::CancelTimer {
        id: TimerId::liveness(&pipeline_id),
    });
    effects.push(Effect::CancelTimer {
        id: TimerId::exit_deferred(&pipeline_id),
    });

    // Transition to cancelled state
    if !pipeline.is_terminal() {
        effects.push(Effect::Emit {
            event: Event::PipelineAdvanced {
                id: pipeline_id.clone(),
                step: "cancelled".to_string(),
            },
        });
    }
    effects.push(Effect::Emit {
        event: Event::StepFailed {
            pipeline_id,
            step: pipeline.step.clone(),
            error: "cancelled".to_string(),
        },
    });

    // Kill session (covers both agent tmux sessions and shell sessions)
    if let Some(session_id) = &pipeline.session_id {
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

/// Build effects to complete a pipeline
pub fn completion_effects(pipeline: &Pipeline) -> Vec<Effect> {
    let pipeline_id = PipelineId::new(&pipeline.id);
    let mut effects = vec![];

    // Cancel liveness and exit-deferred timers
    effects.push(Effect::CancelTimer {
        id: TimerId::liveness(&pipeline_id),
    });
    effects.push(Effect::CancelTimer {
        id: TimerId::exit_deferred(&pipeline_id),
    });

    // Ensure pipeline is in done step with completed status
    if !pipeline.is_terminal() {
        effects.push(Effect::Emit {
            event: Event::PipelineAdvanced {
                id: pipeline_id.clone(),
                step: "done".to_string(),
            },
        });
    }
    effects.push(Effect::Emit {
        event: Event::StepCompleted {
            pipeline_id,
            step: pipeline.step.clone(),
        },
    });

    // Cleanup session if exists
    if let Some(session_id) = &pipeline.session_id {
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
