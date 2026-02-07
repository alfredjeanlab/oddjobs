// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision resolve handler.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use oj_core::{
    AgentRunId, AgentRunStatus, DecisionOption, DecisionSource, Event, JobId, OwnerId, QuestionData,
};

use crate::protocol::Response;

use super::mutations::emit;
use super::ConnectionError;
use super::ListenCtx;

/// Shared context for decision resolution mapping.
///
/// Groups the common parameters needed by both job and agent-run
/// decision resolution paths, avoiding prop-drilling through helpers.
struct DecisionResolveCtx<'a> {
    source: &'a DecisionSource,
    chosen: Option<usize>,
    choices: &'a [usize],
    message: Option<&'a str>,
    decision_id: &'a str,
    options: &'a [DecisionOption],
    question_data: Option<&'a QuestionData>,
}

pub(super) fn handle_decision_resolve(
    ctx: &ListenCtx,
    id: &str,
    chosen: Option<usize>,
    choices: Vec<usize>,
    message: Option<String>,
) -> Result<Response, ConnectionError> {
    let state_guard = ctx.state.lock();

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

    // Validate: choices must match question count and be in range
    if !choices.is_empty() {
        if let Some(ref qd) = decision.question_data {
            if choices.len() != qd.questions.len() {
                return Ok(Response::Error {
                    message: format!(
                        "expected {} choices (one per question), got {}",
                        qd.questions.len(),
                        choices.len()
                    ),
                });
            }
            for (i, &c) in choices.iter().enumerate() {
                let opt_count = qd.questions[i].options.len();
                if c == 0 || c > opt_count {
                    return Ok(Response::Error {
                        message: format!(
                            "choice {} for question {} out of range (1..{})",
                            c,
                            i + 1,
                            opt_count
                        ),
                    });
                }
            }
        }
    }

    // Validate: at least one of chosen, choices, or message must be provided
    if chosen.is_none() && choices.is_empty() && message.is_none() {
        return Ok(Response::Error {
            message: "must provide either a choice or a message (-m)".to_string(),
        });
    }

    let full_id = decision.id.as_str().to_string();
    let job_id = decision.job_id.clone();
    let decision_namespace = decision.namespace.clone();
    let decision_source = decision.source.clone();
    let decision_options = decision.options.clone();
    let decision_question_data = decision.question_data.clone();
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
        choices: choices.clone(),
        message: message.clone(),
        resolved_at_ms,
        namespace: decision_namespace,
    };
    emit(&ctx.event_bus, event)?;

    let resolve_ctx = DecisionResolveCtx {
        source: &decision_source,
        chosen,
        choices: &choices,
        message: message.as_deref(),
        decision_id: &full_id,
        options: &decision_options,
        question_data: decision_question_data.as_ref(),
    };

    // Map chosen option to action based on owner type
    let action_events = match &decision_owner {
        OwnerId::AgentRun(ar_id) => map_decision_to_agent_run_action(
            &resolve_ctx,
            ar_id,
            agent_run_session_id
                .as_deref()
                .or(decision_session_id.as_deref()),
        ),
        OwnerId::Job(_) => {
            map_decision_to_job_action(&resolve_ctx, &job_id, job_step.as_deref())
                .into_iter()
                .collect()
        }
    };

    for action in action_events {
        emit(&ctx.event_bus, action)?;
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
/// - Plan: 1=Accept(clear), 2=Accept(auto), 3=Accept(manual), 4=Revise, 5=Cancel
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
    if matches!(source, DecisionSource::Question) {
        return if choice == options.len() {
            ResolvedAction::Cancel
        } else {
            ResolvedAction::Answer
        };
    }

    match source {
        DecisionSource::Idle => match choice {
            1 => ResolvedAction::Nudge,
            2 => ResolvedAction::Complete,
            3 => ResolvedAction::Cancel,
            4 => ResolvedAction::Dismiss,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Error | DecisionSource::Gate => match choice {
            1 => ResolvedAction::Retry,
            2 => ResolvedAction::Complete,
            3 => ResolvedAction::Cancel,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Approval => match choice {
            1 => ResolvedAction::Approve,
            2 => ResolvedAction::Deny,
            3 => ResolvedAction::Cancel,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Plan => match choice {
            1..=3 => ResolvedAction::Approve,
            4 => ResolvedAction::Freeform,
            5 => ResolvedAction::Cancel,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Question => unreachable!(),
    }
}

/// Map a decision resolution to the appropriate job action event.
fn map_decision_to_job_action(
    ctx: &DecisionResolveCtx,
    job_id: &str,
    job_step: Option<&str>,
) -> Option<Event> {
    let pid = JobId::new(job_id);

    // Multi-question path: choices non-empty means multi-question answer
    if !ctx.choices.is_empty() {
        let resume_msg = build_multi_question_resume_message(ctx);
        return Some(Event::JobResume {
            id: pid,
            message: Some(resume_msg),
            vars: HashMap::new(),
            kill: false,
        });
    }

    let action = resolve_decision_action(ctx.source, ctx.chosen, ctx.options);

    // Plan decisions for jobs route similarly to other approval types
    if matches!(ctx.source, DecisionSource::Plan) {
        return match action {
            ResolvedAction::Approve => Some(Event::JobResume {
                id: pid,
                message: Some(format!("decision {} plan approved", ctx.decision_id)),
                vars: HashMap::new(),
                kill: false,
            }),
            ResolvedAction::Freeform => ctx.message.map(|msg| Event::JobResume {
                id: pid,
                message: Some(format!(
                    "decision {} plan revision: {}",
                    ctx.decision_id, msg
                )),
                vars: HashMap::new(),
                kill: false,
            }),
            ResolvedAction::Cancel => Some(Event::JobCancel { id: pid }),
            _ => None,
        };
    }

    match action {
        ResolvedAction::Freeform => ctx.message.map(|msg| Event::JobResume {
            id: pid,
            message: Some(format!("decision {} freeform: {}", ctx.decision_id, msg)),
            vars: HashMap::new(),
            kill: false,
        }),
        ResolvedAction::Cancel => Some(Event::JobCancel { id: pid }),
        ResolvedAction::Nudge | ResolvedAction::Retry => Some(Event::JobResume {
            id: pid,
            message: Some(build_resume_message(ctx)),
            vars: HashMap::new(),
            kill: false,
        }),
        ResolvedAction::Complete => job_step.map(|step| Event::StepCompleted {
            job_id: pid,
            step: step.to_string(),
        }),
        ResolvedAction::Approve => Some(Event::JobResume {
            id: pid,
            message: Some(format!("decision {} approved", ctx.decision_id)),
            vars: HashMap::new(),
            kill: false,
        }),
        ResolvedAction::Deny => Some(Event::JobCancel { id: pid }),
        ResolvedAction::Answer => Some(Event::JobResume {
            id: pid,
            message: Some(build_question_resume_message(ctx)),
            vars: HashMap::new(),
            kill: false,
        }),
        ResolvedAction::Dismiss => None,
    }
}

/// Map a decision resolution to the appropriate agent run action events.
fn map_decision_to_agent_run_action(
    ctx: &DecisionResolveCtx,
    agent_run_id: &AgentRunId,
    session_id: Option<&str>,
) -> Vec<Event> {
    let ar_id = agent_run_id.clone();

    let send_to_session = |input: String| -> Option<Event> {
        session_id.map(|sid| Event::SessionInput {
            id: oj_core::SessionId::new(sid),
            input,
        })
    };

    // Multi-question path: send concatenated per-question digits
    // e.g., choices [1, 2] → SessionInput "12\n"
    if !ctx.choices.is_empty() {
        let input: String = ctx.choices.iter().map(|c| c.to_string()).collect();
        return send_to_session(format!("{}\n", input))
            .into_iter()
            .collect();
    }

    let action = resolve_decision_action(ctx.source, ctx.chosen, ctx.options);

    match action {
        ResolvedAction::Freeform => {
            if matches!(ctx.source, DecisionSource::Plan) {
                // Plan revision: send Escape to cancel the plan dialog, then
                // send revision feedback as a new user message via AgentRunResume.
                let mut events: Vec<Event> =
                    send_to_session("Escape".to_string()).into_iter().collect();
                if let Some(msg) = ctx.message {
                    events.push(Event::AgentRunResume {
                        id: ar_id,
                        message: Some(msg.to_string()),
                        kill: false,
                    });
                }
                events
            } else {
                ctx.message
                    .map(|msg| Event::AgentRunResume {
                        id: ar_id,
                        message: Some(msg.to_string()),
                        kill: false,
                    })
                    .into_iter()
                    .collect()
            }
        }
        ResolvedAction::Cancel => {
            if matches!(ctx.source, DecisionSource::Plan) {
                // Plan cancel: send Escape to dismiss the dialog, then fail
                let mut events: Vec<Event> =
                    send_to_session("Escape".to_string()).into_iter().collect();
                events.push(Event::AgentRunStatusChanged {
                    id: ar_id,
                    status: AgentRunStatus::Failed,
                    reason: Some(format!("plan rejected via decision {}", ctx.decision_id)),
                });
                events
            } else {
                vec![Event::AgentRunStatusChanged {
                    id: ar_id,
                    status: AgentRunStatus::Failed,
                    reason: Some(format!("cancelled via decision {}", ctx.decision_id)),
                }]
            }
        }
        ResolvedAction::Nudge => {
            let msg = ctx.message.unwrap_or("Please continue with the task.");
            vec![Event::AgentRunResume {
                id: ar_id,
                message: Some(msg.to_string()),
                kill: false,
            }]
        }
        ResolvedAction::Complete => {
            let reason = match ctx.source {
                DecisionSource::Error | DecisionSource::Gate => {
                    format!("skipped via decision {}", ctx.decision_id)
                }
                _ => format!("marked done via decision {}", ctx.decision_id),
            };
            vec![Event::AgentRunStatusChanged {
                id: ar_id,
                status: AgentRunStatus::Completed,
                reason: Some(reason),
            }]
        }
        ResolvedAction::Retry => vec![Event::AgentRunResume {
            id: ar_id,
            message: Some(build_resume_message(ctx)),
            kill: true,
        }],
        ResolvedAction::Approve => {
            if matches!(ctx.source, DecisionSource::Plan) {
                // Plan approval: navigate with arrow keys (Down * N + Enter).
                // Option 1 = cursor already on it → just Enter
                // Option 2 = Down Enter
                // Option 3 = Down Down Enter
                let downs = ctx.chosen.unwrap_or(1) - 1;
                let mut keys = "Down ".repeat(downs);
                keys.push_str("Enter");
                send_to_session(keys).into_iter().collect()
            } else {
                // Permission prompt: send "y" + Enter
                send_to_session("y\n".to_string()).into_iter().collect()
            }
        }
        ResolvedAction::Deny => send_to_session("n\n".to_string()).into_iter().collect(),
        ResolvedAction::Answer => {
            if let Some(c) = ctx.chosen {
                send_to_session(format!("{}\n", c)).into_iter().collect()
            } else if let Some(msg) = ctx.message {
                send_to_session(format!("{}\n", msg)).into_iter().collect()
            } else {
                vec![]
            }
        }
        ResolvedAction::Dismiss => vec![],
    }
}

/// Build a resume message for Question decisions, including the selected option label.
fn build_question_resume_message(ctx: &DecisionResolveCtx) -> String {
    let mut parts = Vec::new();

    if let Some(c) = ctx.chosen {
        let label = ctx
            .options
            .get(c - 1) // 1-indexed to 0-indexed
            .map(|o| o.label.as_str())
            .unwrap_or("unknown");
        parts.push(format!("Selected: {} (option {})", label, c));
    }
    if let Some(m) = ctx.message {
        parts.push(m.to_string());
    }
    if parts.is_empty() {
        parts.push(format!("decision {} resolved", ctx.decision_id));
    }

    parts.join("; ")
}

/// Build a human-readable resume message for multi-question decisions.
fn build_multi_question_resume_message(ctx: &DecisionResolveCtx) -> String {
    if let Some(qd) = ctx.question_data {
        let parts: Vec<String> = ctx
            .choices
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let label = qd
                    .questions
                    .get(i)
                    .and_then(|q| q.options.get(c - 1))
                    .map(|o| o.label.as_str())
                    .unwrap_or("?");
                let header = qd
                    .questions
                    .get(i)
                    .and_then(|q| q.header.as_deref())
                    .unwrap_or("Q");
                format!("{}: {} ({})", header, label, c)
            })
            .collect();
        parts.join("; ")
    } else {
        format!(
            "decision {} resolved: choices [{}]",
            ctx.decision_id,
            ctx.choices
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Build a human-readable resume message from the decision resolution.
fn build_resume_message(ctx: &DecisionResolveCtx) -> String {
    let mut parts = Vec::new();
    if let Some(c) = ctx.chosen {
        parts.push(format!(
            "decision {} resolved: option {}",
            ctx.decision_id, c
        ));
    }
    if let Some(m) = ctx.message {
        if parts.is_empty() {
            parts.push(format!("decision {} resolved: {}", ctx.decision_id, m));
        } else {
            parts.push(format!("message: {}", m));
        }
    }
    parts.join("; ")
}

#[cfg(test)]
#[path = "decisions_tests.rs"]
mod tests;
