// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision resolve handler.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use oj_core::{AgentRunId, AgentRunStatus, DecisionOption, DecisionSource, Event, JobId, OwnerId};
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::mutations::emit;
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
    let job_id = decision.job_id.clone();
    let decision_namespace = decision.namespace.clone();
    let decision_source = decision.source.clone();
    let decision_options = decision.options.clone();
    let decision_owner = decision.owner.clone();
    let decision_session_id = decision.agent_id.clone();

    // Get the job step for StepCompleted events (for job-owned decisions)
    let job_step = state_guard.jobs.get(&job_id).map(|p| p.step.clone());

    // Get agent run session_id (for agent_run-owned decisions)
    let agent_run_session_id = match &decision_owner {
        OwnerId::AgentRun(ar_id) => state_guard
            .agent_runs
            .get(ar_id.as_str())
            .and_then(|r| r.session_id.clone()),
        OwnerId::Job(_) => None,
    };

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
    emit(event_bus, event)?;

    // Map chosen option to action based on owner type
    let action_events = match &decision_owner {
        OwnerId::AgentRun(ar_id) => map_decision_to_agent_run_action(
            &decision_source,
            chosen,
            message.as_deref(),
            &full_id,
            ar_id,
            agent_run_session_id
                .as_deref()
                .or(decision_session_id.as_deref()),
            &decision_options,
        ),
        OwnerId::Job(_) => map_decision_to_job_action(
            &decision_source,
            chosen,
            message.as_deref(),
            &full_id,
            &job_id,
            job_step.as_deref(),
            &decision_options,
        )
        .into_iter()
        .collect(),
    };

    for action in action_events {
        emit(event_bus, action)?;
    }

    Ok(Response::DecisionResolved { id: full_id })
}

/// Intermediate representation of a resolved decision action.
///
/// Captures the intent of a decision resolution independent of whether the
/// target is a job or an agent run.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedAction {
    /// Send a nudge/message to continue working.
    Nudge,
    /// Mark the step/run as complete.
    Complete,
    /// Cancel/abort the job/run.
    Cancel,
    /// Retry the current step (resume for jobs, set Running for agent runs).
    Retry,
    /// Approve a gate/approval.
    Approve,
    /// Deny a gate/approval.
    Deny,
    /// Answer a question with a specific choice.
    Answer,
    /// No action (dismiss or unrecognized choice).
    Dismiss,
    /// Freeform message without a choice.
    Freeform,
}

/// Resolve a decision source + choice into an action.
///
/// Option numbering (1-indexed):
/// - Idle: 1=Nudge, 2=Done, 3=Cancel, 4=Dismiss
/// - Error/Gate: 1=Retry, 2=Skip, 3=Cancel
/// - Approval: 1=Approve, 2=Deny, 3=Cancel
/// - Question: 1..N=user options, N+1=Cancel (dynamic position)
fn resolve_decision_action(
    source: &DecisionSource,
    chosen: Option<usize>,
    options: &[DecisionOption],
) -> ResolvedAction {
    let choice = match chosen {
        Some(c) => c,
        None => return ResolvedAction::Freeform,
    };

    // For Question decisions, Cancel is the last option (dynamic position).
    // For all other decision types, Cancel is always option 3.
    if matches!(source, DecisionSource::Question) {
        return if choice == options.len() {
            ResolvedAction::Cancel
        } else {
            ResolvedAction::Answer
        };
    }
    if choice == 3 {
        return ResolvedAction::Cancel;
    }

    match source {
        DecisionSource::Idle => match choice {
            1 => ResolvedAction::Nudge,
            2 => ResolvedAction::Complete,
            4 => ResolvedAction::Dismiss,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Error | DecisionSource::Gate => match choice {
            1 => ResolvedAction::Retry,
            2 => ResolvedAction::Complete,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Approval => match choice {
            1 => ResolvedAction::Approve,
            2 => ResolvedAction::Deny,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Question => unreachable!(),
    }
}

/// Map a decision resolution to the appropriate job action event.
fn map_decision_to_job_action(
    source: &DecisionSource,
    chosen: Option<usize>,
    message: Option<&str>,
    decision_id: &str,
    job_id: &str,
    job_step: Option<&str>,
    options: &[DecisionOption],
) -> Option<Event> {
    let pid = JobId::new(job_id);
    let action = resolve_decision_action(source, chosen, options);

    match action {
        ResolvedAction::Freeform => message.map(|msg| Event::JobResume {
            id: pid,
            message: Some(format!("decision {} freeform: {}", decision_id, msg)),
            vars: HashMap::new(),
            kill: false,
        }),
        ResolvedAction::Cancel => Some(Event::JobCancel { id: pid }),
        ResolvedAction::Nudge | ResolvedAction::Retry => Some(Event::JobResume {
            id: pid,
            message: Some(build_resume_message(chosen, message, decision_id)),
            vars: HashMap::new(),
            kill: false,
        }),
        ResolvedAction::Complete => job_step.map(|step| Event::StepCompleted {
            job_id: pid,
            step: step.to_string(),
        }),
        ResolvedAction::Approve => Some(Event::JobResume {
            id: pid,
            message: Some(format!("decision {} approved", decision_id)),
            vars: HashMap::new(),
            kill: false,
        }),
        ResolvedAction::Deny => Some(Event::JobCancel { id: pid }),
        ResolvedAction::Answer => Some(Event::JobResume {
            id: pid,
            message: Some(build_question_resume_message(
                chosen,
                message,
                decision_id,
                options,
            )),
            vars: HashMap::new(),
            kill: false,
        }),
        ResolvedAction::Dismiss => None,
    }
}

/// Map a decision resolution to the appropriate agent run action events.
fn map_decision_to_agent_run_action(
    source: &DecisionSource,
    chosen: Option<usize>,
    message: Option<&str>,
    decision_id: &str,
    agent_run_id: &AgentRunId,
    session_id: Option<&str>,
    options: &[DecisionOption],
) -> Vec<Event> {
    let ar_id = agent_run_id.clone();
    let action = resolve_decision_action(source, chosen, options);

    let send_to_session = |input: String| -> Option<Event> {
        session_id.map(|sid| Event::SessionInput {
            id: oj_core::SessionId::new(sid),
            input,
        })
    };

    match action {
        ResolvedAction::Freeform => message
            .and_then(|msg| send_to_session(format!("{}\n", msg)))
            .into_iter()
            .collect(),
        ResolvedAction::Cancel => vec![Event::AgentRunStatusChanged {
            id: ar_id,
            status: AgentRunStatus::Failed,
            reason: Some(format!("cancelled via decision {}", decision_id)),
        }],
        ResolvedAction::Nudge => {
            let msg = message.unwrap_or("Please continue with the task.");
            send_to_session(format!("{}\n", msg)).into_iter().collect()
        }
        ResolvedAction::Complete => {
            let reason = match source {
                DecisionSource::Error | DecisionSource::Gate => {
                    format!("skipped via decision {}", decision_id)
                }
                _ => format!("marked done via decision {}", decision_id),
            };
            vec![Event::AgentRunStatusChanged {
                id: ar_id,
                status: AgentRunStatus::Completed,
                reason: Some(reason),
            }]
        }
        ResolvedAction::Retry => vec![Event::AgentRunStatusChanged {
            id: ar_id,
            status: AgentRunStatus::Running,
            reason: Some(format!("retry via decision {}", decision_id)),
        }],
        ResolvedAction::Approve => send_to_session("y\n".to_string()).into_iter().collect(),
        ResolvedAction::Deny => send_to_session("n\n".to_string()).into_iter().collect(),
        ResolvedAction::Answer => {
            if let Some(c) = chosen {
                send_to_session(format!("{}\n", c)).into_iter().collect()
            } else if let Some(msg) = message {
                send_to_session(format!("{}\n", msg)).into_iter().collect()
            } else {
                vec![]
            }
        }
        ResolvedAction::Dismiss => vec![],
    }
}

/// Build a resume message for Question decisions, including the selected option label.
fn build_question_resume_message(
    chosen: Option<usize>,
    message: Option<&str>,
    decision_id: &str,
    options: &[DecisionOption],
) -> String {
    let mut parts = Vec::new();

    if let Some(c) = chosen {
        let label = options
            .get(c - 1) // 1-indexed to 0-indexed
            .map(|o| o.label.as_str())
            .unwrap_or("unknown");
        parts.push(format!("Selected: {} (option {})", label, c));
    }
    if let Some(m) = message {
        parts.push(m.to_string());
    }
    if parts.is_empty() {
        parts.push(format!("decision {} resolved", decision_id));
    }

    parts.join("; ")
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
