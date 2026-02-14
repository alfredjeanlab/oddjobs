// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision resolve handler.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use oj_core::{
    CrewId, CrewStatus, DecisionOption, DecisionSource, Event, JobId, OwnerId, QuestionData,
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
    /// Raw 1-indexed choices from the request (single-element for single-choice,
    /// multi-element for multi-question, empty for freeform-only).
    choices: &'a [usize],
    message: Option<&'a str>,
    decision_id: &'a str,
    options: &'a [DecisionOption],
    questions: Option<&'a QuestionData>,
}

impl DecisionResolveCtx<'_> {
    /// Single-choice answer (for non-multi-question decisions with exactly one choice).
    fn chosen(&self) -> Option<usize> {
        if self.choices.len() == 1 && !self.is_multi_question() {
            Some(self.choices[0])
        } else {
            None
        }
    }

    /// Whether this is a multi-question resolution.
    fn is_multi_question(&self) -> bool {
        self.choices.len() > 1 && self.questions.is_some()
    }
}

pub(super) fn handle_decision_resolve(
    ctx: &ListenCtx,
    id: &str,
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
        return Ok(Response::Error { message: format!("decision {} is already resolved", id) });
    }

    // Validate choices
    let is_multi_question = decision.questions.is_some() && choices.len() > 1;
    if choices.len() == 1 && !is_multi_question {
        let choice = choices[0];
        if choice == 0 || choice > decision.options.len() {
            return Ok(Response::Error {
                message: format!("choice {} out of range (1..{})", choice, decision.options.len()),
            });
        }
    } else if let (true, Some(qd)) = (is_multi_question, decision.questions.as_ref()) {
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
            // Allow opt_count + 1 for the "Other" (custom response) option
            if c == 0 || c > opt_count + 1 {
                return Ok(Response::Error {
                    message: format!(
                        "choice {} for question {} out of range (1..{})",
                        c,
                        i + 1,
                        opt_count + 1
                    ),
                });
            }
        }
    } else if choices.is_empty() && message.is_none() {
        return Ok(Response::Error {
            message: "must provide either a choice or a message (-m)".to_string(),
        });
    }

    let decision_id = decision.id.clone();
    let decision_namespace = decision.project.clone();
    let decision_source = decision.source.clone();
    let decision_options = decision.options.clone();
    let decision_questions = decision.questions.clone();
    let decision_owner = decision.owner.clone();
    let decision_agent_id = decision.agent_id.to_string();

    // Get the job step for StepCompleted events (for job-owned decisions)
    let job_id = decision_owner.as_job().map(|id| id.to_string()).unwrap_or_default();
    let job_step = state_guard.jobs.get(&job_id).map(|p| p.step.clone());

    // Get crew agent_id (prefer live crew state over decision snapshot)
    let crew_agent_id = decision_owner
        .as_crew()
        .and_then(|crew_id| state_guard.crew.get(crew_id.as_str()))
        .and_then(|r| r.agent_id.clone());

    drop(state_guard);

    let resolved_at_ms =
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;

    let resolve_ctx = DecisionResolveCtx {
        source: &decision_source,
        choices: &choices,
        message: message.as_deref(),
        decision_id: &decision_id,
        options: &decision_options,
        questions: decision_questions.as_ref(),
    };

    // Emit DecisionResolved
    let resolved_choices = if resolve_ctx.is_multi_question() {
        choices.clone()
    } else {
        resolve_ctx.chosen().map(|c| vec![c]).unwrap_or_default()
    };
    let event = Event::DecisionResolved {
        id: decision_id.clone(),
        choices: resolved_choices,
        message: message.clone(),
        resolved_at_ms,
        project: decision_namespace,
    };
    emit(&ctx.event_bus, event)?;

    // Map chosen option to action based on owner type
    let action_events = match &decision_owner {
        OwnerId::Crew(run_id) => map_decision_to_crew_action(
            &resolve_ctx,
            run_id,
            Some(crew_agent_id.as_deref().unwrap_or(&decision_agent_id)),
        ),
        OwnerId::Job(_) => map_decision_to_job_action(
            &resolve_ctx,
            &job_id,
            job_step.as_deref(),
            Some(&decision_agent_id),
        ),
    };

    for event in action_events {
        emit(&ctx.event_bus, event)?;
    }

    Ok(Response::DecisionResolved { id: decision_id })
}

/// Intermediate representation of a resolved decision action.
///
/// Captures the intent of a decision resolution independent of whether the
/// target is a job or an crew.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedAction {
    /// Send a nudge/message to continue working.
    Nudge,
    /// Mark the step/run as complete.
    Complete,
    /// Cancel/abort the job/run.
    Cancel,
    /// Retry the current step (resume for jobs, set Running for crew).
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
/// - Error/Dead: 1=Retry, 2=Skip, 3=Cancel, 4=Dismiss
/// - Gate: 1=Retry, 2=Skip, 3=Cancel
/// - Approval: 1=Approve, 2=Deny, 3=Cancel, 4=Dismiss
/// - Question: 1..N=user options, N+1=Other, N+2=Cancel, N+3=Dismiss (dynamic positions)
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

    // For Question decisions: Other is third-to-last, Cancel is second-to-last,
    // Dismiss is last (dynamic positions).
    if matches!(source, DecisionSource::Question) {
        return if choice == options.len() {
            ResolvedAction::Dismiss
        } else if choice == options.len() - 1 {
            ResolvedAction::Cancel
        } else if choice == options.len() - 2 {
            ResolvedAction::Freeform
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
        DecisionSource::Error | DecisionSource::Dead => match choice {
            1 => ResolvedAction::Retry,
            2 => ResolvedAction::Complete,
            3 => ResolvedAction::Cancel,
            4 => ResolvedAction::Dismiss,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Gate => match choice {
            1 => ResolvedAction::Retry,
            2 => ResolvedAction::Complete,
            3 => ResolvedAction::Cancel,
            _ => ResolvedAction::Dismiss,
        },
        DecisionSource::Approval => match choice {
            1 => ResolvedAction::Approve,
            2 => ResolvedAction::Deny,
            3 => ResolvedAction::Cancel,
            4 => ResolvedAction::Dismiss,
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

/// Map a decision resolution to the appropriate job action event(s).
fn map_decision_to_job_action(
    ctx: &DecisionResolveCtx,
    job_id: &str,
    job_step: Option<&str>,
    agent_id: Option<&str>,
) -> Vec<Event> {
    let pid = JobId::from_string(job_id);

    let respond_to_agent = |response: oj_core::PromptResponse| -> Option<Event> {
        agent_id.map(|aid| Event::AgentRespond { id: oj_core::AgentId::from_string(aid), response })
    };

    // Multi-question path
    if ctx.is_multi_question() {
        let resume_msg = build_multi_question_resume_message(ctx);
        return vec![Event::JobResume {
            id: pid,
            message: Some(resume_msg),
            vars: HashMap::new(),
            kill: false,
        }];
    }

    let action = resolve_decision_action(ctx.source, ctx.chosen(), ctx.options);

    // Plan decisions use the structured respond API to interact with the
    // agent's plan dialog instead of sending raw keyboard sequences.
    if matches!(ctx.source, DecisionSource::Plan) {
        return match action {
            ResolvedAction::Approve => {
                let option = ctx.chosen().unwrap_or(1) as u32;
                // Transition step_status from Waiting → Running so the
                // watcher's subsequent AgentIdle can set a grace timer.
                let mut events = Vec::new();
                if let Some(step) = job_step {
                    events.push(Event::StepStarted {
                        job_id: pid.clone(),
                        step: step.to_string(),
                        agent_id: None,
                        agent_name: None,
                    });
                }
                events.extend(respond_to_agent(oj_core::PromptResponse {
                    accept: None,
                    option: Some(option),
                    text: None,
                }));
                events
            }
            ResolvedAction::Freeform => {
                // Plan revision: send feedback text via the respond API.
                let mut events = Vec::new();
                let text = ctx.message.map(|s| s.to_string());
                events.extend(respond_to_agent(oj_core::PromptResponse {
                    accept: None,
                    option: None,
                    text: text.clone(),
                }));
                if let Some(msg) = text {
                    events.push(Event::JobResume {
                        id: pid,
                        message: Some(msg),
                        vars: HashMap::new(),
                        kill: false,
                    });
                }
                events
            }
            ResolvedAction::Cancel => {
                // Plan cancel: reject via respond API, then cancel job.
                let mut events: Vec<Event> = respond_to_agent(oj_core::PromptResponse {
                    accept: Some(false),
                    option: None,
                    text: None,
                })
                .into_iter()
                .collect();
                events.push(Event::JobCancel { id: pid });
                events
            }
            _ => vec![],
        };
    }

    match action {
        ResolvedAction::Freeform => ctx
            .message
            .map(|msg| Event::JobResume {
                id: pid,
                message: Some(msg.to_string()),
                vars: HashMap::new(),
                kill: false,
            })
            .into_iter()
            .collect(),
        ResolvedAction::Cancel => vec![Event::JobCancel { id: pid }],
        ResolvedAction::Nudge | ResolvedAction::Retry => vec![Event::JobResume {
            id: pid,
            message: Some(build_resume_message(ctx)),
            vars: HashMap::new(),
            kill: false,
        }],
        ResolvedAction::Complete => job_step
            .map(|step| Event::StepCompleted { job_id: pid, step: step.to_string() })
            .into_iter()
            .collect(),
        ResolvedAction::Approve => vec![Event::JobResume {
            id: pid,
            message: Some("Approved.".to_string()),
            vars: HashMap::new(),
            kill: false,
        }],
        ResolvedAction::Deny => vec![Event::JobCancel { id: pid }],
        ResolvedAction::Answer => vec![Event::JobResume {
            id: pid,
            message: Some(build_question_resume_message(ctx)),
            vars: HashMap::new(),
            kill: false,
        }],
        ResolvedAction::Dismiss => vec![],
    }
}

/// Map a decision resolution to the appropriate crew action events.
fn map_decision_to_crew_action(
    ctx: &DecisionResolveCtx,
    crew_id: &CrewId,
    agent_id: Option<&str>,
) -> Vec<Event> {
    let run_id = crew_id.clone();

    let send_to_agent = |input: String| -> Option<Event> {
        agent_id.map(|aid| Event::AgentInput { id: oj_core::AgentId::from_string(aid), input })
    };
    let respond_to_agent = |response: oj_core::PromptResponse| -> Option<Event> {
        agent_id.map(|aid| Event::AgentRespond { id: oj_core::AgentId::from_string(aid), response })
    };

    // Multi-question path: send concatenated per-question digits
    // e.g., choices [1, 2] → AgentInput "12\n"
    if ctx.is_multi_question() {
        let input: String = ctx.choices.iter().map(|c| c.to_string()).collect();
        return send_to_agent(format!("{}\n", input)).into_iter().collect();
    }

    let action = resolve_decision_action(ctx.source, ctx.chosen(), ctx.options);

    match action {
        ResolvedAction::Freeform => {
            if matches!(ctx.source, DecisionSource::Plan) {
                // Plan revision: send feedback text via the respond API.
                let mut events = Vec::new();
                let text = ctx.message.map(|s| s.to_string());
                events.extend(respond_to_agent(oj_core::PromptResponse {
                    accept: None,
                    option: None,
                    text: text.clone(),
                }));
                if let Some(msg) = text {
                    events.push(Event::CrewResume { id: run_id, message: Some(msg), kill: false });
                }
                events
            } else if matches!(ctx.source, DecisionSource::Question) {
                // Question "Other": send custom text as session input since
                // the agent is waiting at an AskUserQuestion prompt.
                ctx.message
                    .and_then(|msg| send_to_agent(format!("{}\n", msg)))
                    .into_iter()
                    .collect()
            } else {
                ctx.message
                    .map(|msg| Event::CrewResume {
                        id: run_id,
                        message: Some(msg.to_string()),
                        kill: false,
                    })
                    .into_iter()
                    .collect()
            }
        }
        ResolvedAction::Cancel => {
            if matches!(ctx.source, DecisionSource::Plan) {
                // Plan cancel: reject via respond API, then fail
                let mut events: Vec<Event> = respond_to_agent(oj_core::PromptResponse {
                    accept: Some(false),
                    option: None,
                    text: None,
                })
                .into_iter()
                .collect();
                events.push(Event::CrewUpdated {
                    id: run_id,
                    status: CrewStatus::Failed,
                    reason: Some(format!("plan rejected via decision {}", ctx.decision_id)),
                });
                events
            } else {
                vec![Event::CrewUpdated {
                    id: run_id,
                    status: CrewStatus::Failed,
                    reason: Some(format!("cancelled via decision {}", ctx.decision_id)),
                }]
            }
        }
        ResolvedAction::Nudge => {
            let msg = ctx.message.unwrap_or("Please continue with the task.");
            vec![Event::CrewResume { id: run_id, message: Some(msg.to_string()), kill: false }]
        }
        ResolvedAction::Complete => {
            let reason = match ctx.source {
                DecisionSource::Error | DecisionSource::Dead | DecisionSource::Gate => {
                    format!("skipped via decision {}", ctx.decision_id)
                }
                _ => format!("marked done via decision {}", ctx.decision_id),
            };
            vec![Event::CrewUpdated {
                id: run_id,
                status: CrewStatus::Completed,
                reason: Some(reason),
            }]
        }
        ResolvedAction::Retry => vec![Event::CrewResume {
            id: run_id,
            message: Some(build_resume_message(ctx)),
            kill: true,
        }],
        ResolvedAction::Approve => {
            if matches!(ctx.source, DecisionSource::Plan) {
                // Plan approval: use structured respond API with option number.
                let option = ctx.chosen().unwrap_or(1) as u32;
                respond_to_agent(oj_core::PromptResponse {
                    accept: None,
                    option: Some(option),
                    text: None,
                })
                .into_iter()
                .collect()
            } else {
                // Permission prompt: send "y" + Enter
                send_to_agent("y\n".to_string()).into_iter().collect()
            }
        }
        ResolvedAction::Deny => send_to_agent("n\n".to_string()).into_iter().collect(),
        ResolvedAction::Answer => {
            if let Some(c) = ctx.chosen() {
                send_to_agent(format!("{}\n", c)).into_iter().collect()
            } else if let Some(msg) = ctx.message {
                send_to_agent(format!("{}\n", msg)).into_iter().collect()
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

    if let Some(c) = ctx.chosen() {
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

    parts.join("; ")
}

/// Build a human-readable resume message for multi-question decisions.
fn build_multi_question_resume_message(ctx: &DecisionResolveCtx) -> String {
    if let Some(qd) = ctx.questions {
        let parts: Vec<String> = ctx
            .choices
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let header = qd.questions.get(i).and_then(|q| q.header.as_deref()).unwrap_or("Q");
                let opt_count = qd.questions.get(i).map(|q| q.options.len()).unwrap_or(0);
                if c == opt_count + 1 {
                    // "Other" choice — include freeform message if available
                    if let Some(msg) = ctx.message {
                        format!("{}: Other - {}", header, msg)
                    } else {
                        format!("{}: Other", header)
                    }
                } else {
                    let label = qd
                        .questions
                        .get(i)
                        .and_then(|q| q.options.get(c - 1))
                        .map(|o| o.label.as_str())
                        .unwrap_or("?");
                    format!("{}: {} ({})", header, label, c)
                }
            })
            .collect();
        parts.join("; ")
    } else {
        format!(
            "Selected: choices [{}]",
            ctx.choices.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", ")
        )
    }
}

/// Build a resume message from the decision resolution.
///
/// Sends the user's message as-is when provided, falling back to a default
/// nudge prompt. Decision metadata is not useful to agents, so we omit it.
fn build_resume_message(ctx: &DecisionResolveCtx) -> String {
    ctx.message.unwrap_or("Please continue with the task.").to_string()
}

#[cfg(test)]
#[path = "decisions_tests.rs"]
mod tests;
