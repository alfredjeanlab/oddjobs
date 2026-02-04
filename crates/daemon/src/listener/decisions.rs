// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision resolve handler.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use oj_core::{DecisionSource, Event, PipelineId};
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::ConnectionError;

pub(super) fn handle_decision_resolve(
    id: &str,
    chosen: Option<usize>,
    message: Option<String>,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    let state_guard = state.lock();

    // Find decision by ID or prefix
    let decision = state_guard
        .get_decision(id)
        .ok_or_else(|| ConnectionError::Internal(format!("decision not found: {}", id)))?;

    // Validate: must be unresolved
    if decision.is_resolved() {
        return Ok(Response::Error {
            message: format!("decision {} is already resolved", id),
        });
    }

    // Validate: choice must be in range if provided
    if let Some(choice) = chosen {
        if choice == 0 || choice > decision.options.len() {
            return Ok(Response::Error {
                message: format!(
                    "choice {} out of range (1..{})",
                    choice,
                    decision.options.len()
                ),
            });
        }
    }

    // Validate: at least one of chosen or message must be provided
    if chosen.is_none() && message.is_none() {
        return Ok(Response::Error {
            message: "must provide either a choice or a message (-m)".to_string(),
        });
    }

    let full_id = decision.id.as_str().to_string();
    let pipeline_id = decision.pipeline_id.clone();
    let decision_namespace = decision.namespace.clone();
    let decision_source = decision.source.clone();

    // Get the pipeline step for StepCompleted events
    let pipeline_step = state_guard
        .pipelines
        .get(&pipeline_id)
        .map(|p| p.step.clone());
    drop(state_guard);

    let resolved_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Emit DecisionResolved
    let event = Event::DecisionResolved {
        id: full_id.clone(),
        chosen,
        message: message.clone(),
        resolved_at_ms,
        namespace: decision_namespace,
    };
    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    // Map chosen option to pipeline action based on decision source
    let action_event = map_decision_to_action(
        &decision_source,
        chosen,
        message.as_deref(),
        &full_id,
        &pipeline_id,
        pipeline_step.as_deref(),
    );

    if let Some(action) = action_event {
        event_bus
            .send(action)
            .map_err(|_| ConnectionError::WalError)?;
    }

    Ok(Response::DecisionResolved { id: full_id })
}

/// Map a decision resolution to the appropriate pipeline action event.
///
/// Option numbering (1-indexed):
/// - Idle: 1=Nudge, 2=Done, 3=Cancel
/// - Error/Gate: 1=Retry, 2=Skip, 3=Cancel
/// - Approval: 1=Approve, 2=Deny, 3=Cancel
fn map_decision_to_action(
    source: &DecisionSource,
    chosen: Option<usize>,
    message: Option<&str>,
    decision_id: &str,
    pipeline_id: &str,
    pipeline_step: Option<&str>,
) -> Option<Event> {
    let pid = PipelineId::new(pipeline_id);

    // Handle based on whether a choice was provided
    let choice = match chosen {
        Some(c) => c,
        None => {
            // No choice provided - if there's a message, treat as freeform nudge
            return message.map(|msg| Event::PipelineResume {
                id: pid,
                message: Some(format!("decision {} freeform: {}", decision_id, msg)),
                vars: HashMap::new(),
            });
        }
    };

    // Cancel is always option 3 for all decision types
    if choice == 3 {
        return Some(Event::PipelineCancel { id: pid });
    }

    match source {
        DecisionSource::Idle => match choice {
            // 1 = Nudge: resume with message
            1 => Some(Event::PipelineResume {
                id: pid,
                message: Some(build_resume_message(chosen, message, decision_id)),
                vars: HashMap::new(),
            }),
            // 2 = Done: mark step complete
            2 => pipeline_step.map(|step| Event::StepCompleted {
                pipeline_id: pid,
                step: step.to_string(),
            }),
            _ => None,
        },
        DecisionSource::Error | DecisionSource::Gate => match choice {
            // 1 = Retry: resume (runtime will re-run)
            1 => Some(Event::PipelineResume {
                id: pid,
                message: Some(build_resume_message(chosen, message, decision_id)),
                vars: HashMap::new(),
            }),
            // 2 = Skip: mark step complete
            2 => pipeline_step.map(|step| Event::StepCompleted {
                pipeline_id: pid,
                step: step.to_string(),
            }),
            _ => None,
        },
        DecisionSource::Approval => match choice {
            // 1 = Approve: resume with approval message
            1 => Some(Event::PipelineResume {
                id: pid,
                message: Some(format!("decision {} approved", decision_id)),
                vars: HashMap::new(),
            }),
            // 2 = Deny: cancel (deny usually means abort)
            2 => Some(Event::PipelineCancel { id: pid }),
            _ => None,
        },
        DecisionSource::Question => {
            // Question decisions: always resume with the chosen option
            Some(Event::PipelineResume {
                id: pid,
                message: Some(build_resume_message(chosen, message, decision_id)),
                vars: HashMap::new(),
            })
        }
    }
}

/// Build a human-readable resume message from the decision resolution.
fn build_resume_message(chosen: Option<usize>, message: Option<&str>, decision_id: &str) -> String {
    let mut parts = Vec::new();
    if let Some(c) = chosen {
        parts.push(format!("decision {} resolved: option {}", decision_id, c));
    }
    if let Some(m) = message {
        if parts.is_empty() {
            parts.push(format!("decision {} resolved: {}", decision_id, m));
        } else {
            parts.push(format!("message: {}", m));
        }
    }
    parts.join("; ")
}

#[cfg(test)]
#[path = "decisions_tests.rs"]
mod tests;
