// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session monitoring for agent jobs.
//!
//! Handles detection of agent state from session logs and triggers
//! appropriate actions (nudge, resume, escalate, etc.).

use crate::decision_builder::{EscalationDecisionBuilder, EscalationTrigger};
use crate::RuntimeError;
use oj_core::{
    AgentError, AgentState, Effect, Event, Job, JobId, PromptType, QuestionData, SessionId, TimerId,
};
use oj_runbook::{ActionConfig, AgentAction, AgentDef, ErrorType, RunDirective, Runbook};
use std::collections::HashMap;
use std::time::Duration;

/// Parse a duration string like "30s", "5m", "1h" into a Duration
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".to_string());
    }

    // Find the numeric prefix
    let (num_str, suffix) = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| (&s[..i], &s[i..]))
        .unwrap_or((s, ""));

    let num: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number in duration: {}", s))?;

    let multiplier = match suffix.trim() {
        "ms" | "millis" | "millisecond" | "milliseconds" => {
            return Ok(Duration::from_millis(num));
        }
        "" | "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86400,
        other => return Err(format!("unknown duration suffix: {}", other)),
    };

    Ok(Duration::from_secs(num * multiplier))
}

/// Normalized monitor state for unified handling of AgentState and SessionState.
///
/// Both AgentState (from file watchers) and SessionState (from session logs) represent
/// the same conceptual states but with different type representations. This enum
/// normalizes them for unified handling.
#[derive(Debug, Clone)]
pub enum MonitorState {
    /// Agent is actively working
    Working,
    /// Agent is idle, waiting for input
    WaitingForInput,
    /// Agent is showing a prompt (permission, plan approval, etc.)
    Prompting {
        prompt_type: PromptType,
        question_data: Option<QuestionData>,
    },
    /// Agent encountered an error
    Failed {
        message: String,
        error_type: Option<ErrorType>,
    },
    /// Agent process exited
    Exited,
    /// Session terminated unexpectedly
    Gone,
}

impl MonitorState {
    /// Create from AgentState
    pub fn from_agent_state(state: &AgentState) -> Self {
        match state {
            AgentState::Working => MonitorState::Working,
            AgentState::WaitingForInput => MonitorState::WaitingForInput,
            AgentState::Failed(failure) => MonitorState::Failed {
                message: failure.to_string(),
                error_type: agent_failure_to_error_type(failure),
            },
            AgentState::Exited { .. } => MonitorState::Exited,
            AgentState::SessionGone => MonitorState::Gone,
        }
    }
}

/// Convert an AgentError to an error type
pub fn agent_failure_to_error_type(failure: &AgentError) -> Option<ErrorType> {
    match failure {
        AgentError::Unauthorized => Some(ErrorType::Unauthorized),
        AgentError::OutOfCredits => Some(ErrorType::OutOfCredits),
        AgentError::NoInternet => Some(ErrorType::NoInternet),
        AgentError::RateLimited => Some(ErrorType::RateLimited),
        AgentError::Other(_) => None,
    }
}

/// Get the current agent definition for a job step
pub fn get_agent_def<'a>(runbook: &'a Runbook, job: &Job) -> Result<&'a AgentDef, RuntimeError> {
    let job_def = runbook
        .get_job(&job.kind)
        .ok_or_else(|| RuntimeError::JobDefNotFound(job.kind.clone()))?;

    let step_def = job_def
        .get_step(&job.step)
        .ok_or_else(|| RuntimeError::JobNotFound(format!("step {} not found", job.step)))?;

    // Extract agent name from run directive
    let agent_name = match &step_def.run {
        RunDirective::Agent { agent, .. } => agent,
        _ => {
            return Err(RuntimeError::InvalidRunDirective {
                context: format!("step {}", job.step),
                directive: "not an agent step".to_string(),
            })
        }
    };

    runbook
        .get_agent(agent_name)
        .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.clone()))
}

/// Build effects for an agent action (nudge, recover, escalate, etc.)
pub fn build_action_effects(
    job: &Job,
    agent_def: &AgentDef,
    action_config: &ActionConfig,
    trigger: &str,
    input: &HashMap<String, String>,
    question_data: Option<&QuestionData>,
) -> Result<ActionEffects, RuntimeError> {
    let action = action_config.action();
    let message = action_config.message();
    let job_id = JobId::new(&job.id);

    tracing::info!(
        job_id = %job.id,
        trigger = trigger,
        action = ?action,
        "building agent action effects"
    );

    match action {
        AgentAction::Nudge => {
            let session_id = job
                .session_id
                .as_ref()
                .ok_or_else(|| RuntimeError::JobNotFound("no session".into()))?;

            let nudge_message = message.unwrap_or("Please continue with the task.");
            Ok(ActionEffects::Nudge {
                effects: vec![Effect::SendToSession {
                    session_id: SessionId::new(session_id),
                    input: format!("{}\n", nudge_message),
                }],
            })
        }

        AgentAction::Done => Ok(ActionEffects::AdvanceJob),

        AgentAction::Fail => Ok(ActionEffects::FailJob {
            error: trigger.to_string(),
        }),

        AgentAction::Resume => {
            // Build modified input for re-spawn
            let mut new_inputs = input.clone();
            // Determine whether to use --resume: yes when no message or append mode
            let use_resume = message.is_none() || action_config.append();

            if let Some(msg) = message {
                if action_config.append() && use_resume {
                    // Message will be passed as argument to --resume
                    new_inputs.insert("resume_message".to_string(), msg.to_string());
                } else {
                    // Replace mode: full prompt replacement, no --resume
                    new_inputs.insert("prompt".to_string(), msg.to_string());
                }
            }

            // Look up previous Claude session ID from step history
            let resume_session_id = job
                .step_history
                .iter()
                .rfind(|r| r.name == job.step)
                .and_then(|r| r.agent_id.clone());

            Ok(ActionEffects::Resume {
                kill_session: job.session_id.clone(),
                agent_name: agent_def.name.clone(),
                input: new_inputs,
                resume_session_id: if use_resume { resume_session_id } else { None },
            })
        }

        AgentAction::Escalate => {
            tracing::warn!(
                job_id = %job.id,
                trigger = trigger,
                message = ?message,
                "escalating to human — creating decision"
            );

            // Determine escalation trigger type from the trigger string
            let escalation_trigger = match trigger {
                "idle" | "on_idle" => EscalationTrigger::Idle,
                "dead" | "on_dead" | "exit" | "exited" => {
                    EscalationTrigger::Dead { exit_code: None }
                }
                "error" | "on_error" => EscalationTrigger::Error {
                    error_type: "unknown".to_string(),
                    message: message.unwrap_or("").to_string(),
                },
                "prompt:question" => EscalationTrigger::Question {
                    question_data: question_data.cloned(),
                },
                "prompt" | "on_prompt" => EscalationTrigger::Prompt {
                    prompt_type: "permission".to_string(),
                },
                t if t.ends_with("_exhausted") => {
                    // Handle "idle_exhausted", "error_exhausted",
                    // "prompt:question_exhausted" etc.
                    let base = t.trim_end_matches("_exhausted");
                    match base {
                        "idle" => EscalationTrigger::Idle,
                        "prompt:question" => EscalationTrigger::Question {
                            question_data: question_data.cloned(),
                        },
                        "error" => EscalationTrigger::Error {
                            error_type: "exhausted".to_string(),
                            message: message.unwrap_or("").to_string(),
                        },
                        _ => EscalationTrigger::Dead { exit_code: None },
                    }
                }
                _ => EscalationTrigger::Idle, // fallback
            };

            let (_decision_id, decision_event) = EscalationDecisionBuilder::for_job(
                job_id.clone(),
                job.name.clone(),
                escalation_trigger,
            )
            .agent_id(job.session_id.clone().unwrap_or_default())
            .namespace(job.namespace.clone())
            .build();

            let effects = vec![
                // Emit DecisionCreated (this also sets job to Waiting)
                Effect::Emit {
                    event: decision_event,
                },
                // Desktop notification on decision created
                Effect::Notify {
                    title: format!("Decision needed: {}", job.name),
                    message: format!("Job requires attention ({})", trigger),
                },
                // Cancel exit-deferred timer (agent may still be alive)
                Effect::CancelTimer {
                    id: TimerId::exit_deferred(&job_id),
                },
            ];

            Ok(ActionEffects::Escalate { effects })
        }

        AgentAction::Gate => {
            let command = action_config
                .run()
                .ok_or_else(|| RuntimeError::InvalidRunDirective {
                    context: format!("job {}", job.id),
                    directive: "gate action requires a 'run' field".to_string(),
                })?
                .to_string();

            Ok(ActionEffects::Gate { command })
        }
    }
}

/// Build effects for an agent action on a standalone agent run.
pub fn build_action_effects_for_agent_run(
    agent_run: &oj_core::AgentRun,
    agent_def: &AgentDef,
    action_config: &ActionConfig,
    trigger: &str,
    input: &HashMap<String, String>,
    _question_data: Option<&QuestionData>,
) -> Result<ActionEffects, RuntimeError> {
    let action = action_config.action();
    let message = action_config.message();
    let agent_run_id = oj_core::AgentRunId::new(&agent_run.id);

    tracing::info!(
        agent_run_id = %agent_run.id,
        trigger = trigger,
        action = ?action,
        "building agent run action effects"
    );

    match action {
        AgentAction::Nudge => {
            let session_id = agent_run
                .session_id
                .as_ref()
                .ok_or_else(|| RuntimeError::InvalidRequest("no session for nudge".into()))?;

            let nudge_message = message.unwrap_or("Please continue with the task.");
            Ok(ActionEffects::Nudge {
                effects: vec![Effect::SendToSession {
                    session_id: SessionId::new(session_id),
                    input: format!("{}\n", nudge_message),
                }],
            })
        }

        AgentAction::Done => Ok(ActionEffects::CompleteAgentRun),

        AgentAction::Fail => Ok(ActionEffects::FailAgentRun {
            error: trigger.to_string(),
        }),

        AgentAction::Resume => {
            let mut new_inputs = input.clone();
            let use_resume = message.is_none() || action_config.append();

            if let Some(msg) = message {
                if action_config.append() && use_resume {
                    new_inputs.insert("resume_message".to_string(), msg.to_string());
                } else {
                    new_inputs.insert("prompt".to_string(), msg.to_string());
                }
            }

            // Look up previous Claude session ID from agent run
            let resume_session_id = agent_run.agent_id.clone();

            Ok(ActionEffects::Resume {
                kill_session: agent_run.session_id.clone(),
                agent_name: agent_def.name.clone(),
                input: new_inputs,
                resume_session_id: if use_resume { resume_session_id } else { None },
            })
        }

        AgentAction::Escalate => {
            tracing::warn!(
                agent_run_id = %agent_run.id,
                trigger = trigger,
                message = ?message,
                "escalating standalone agent to human — creating decision"
            );

            // Determine escalation trigger type from the trigger string
            let escalation_trigger = match trigger {
                "idle" | "on_idle" => EscalationTrigger::Idle,
                "dead" | "on_dead" | "exit" | "exited" => {
                    EscalationTrigger::Dead { exit_code: None }
                }
                "error" | "on_error" => EscalationTrigger::Error {
                    error_type: "unknown".to_string(),
                    message: message.unwrap_or("").to_string(),
                },
                "prompt:question" => EscalationTrigger::Question {
                    question_data: _question_data.cloned(),
                },
                "prompt" | "on_prompt" => EscalationTrigger::Prompt {
                    prompt_type: "permission".to_string(),
                },
                t if t.ends_with("_exhausted") => {
                    let base = t.trim_end_matches("_exhausted");
                    match base {
                        "idle" => EscalationTrigger::Idle,
                        "prompt:question" => EscalationTrigger::Question {
                            question_data: _question_data.cloned(),
                        },
                        "error" => EscalationTrigger::Error {
                            error_type: "exhausted".to_string(),
                            message: message.unwrap_or("").to_string(),
                        },
                        _ => EscalationTrigger::Dead { exit_code: None },
                    }
                }
                _ => EscalationTrigger::Idle, // fallback
            };

            let (_decision_id, decision_event) = EscalationDecisionBuilder::for_agent_run(
                agent_run_id.clone(),
                agent_run.command_name.clone(),
                escalation_trigger,
            )
            .agent_id(agent_run.agent_id.clone().unwrap_or_default())
            .namespace(agent_run.namespace.clone())
            .build();

            let effects = vec![
                // Emit DecisionCreated
                Effect::Emit {
                    event: decision_event,
                },
                // Desktop notification on decision created
                Effect::Notify {
                    title: format!("Decision needed: {}", agent_run.command_name),
                    message: format!("Agent requires attention ({})", trigger),
                },
                // Emit AgentRunStatusChanged to Escalated
                Effect::Emit {
                    event: Event::AgentRunStatusChanged {
                        id: agent_run_id.clone(),
                        status: oj_core::AgentRunStatus::Escalated,
                        reason: Some(trigger.to_string()),
                    },
                },
                // Cancel exit-deferred timer (agent is still alive; liveness continues)
                Effect::CancelTimer {
                    id: TimerId::exit_deferred_agent_run(&agent_run_id),
                },
            ];

            Ok(ActionEffects::EscalateAgentRun { effects })
        }

        AgentAction::Gate => {
            let command = action_config
                .run()
                .ok_or_else(|| RuntimeError::InvalidRunDirective {
                    context: format!("agent_run {}", agent_run.id),
                    directive: "gate action requires a 'run' field".to_string(),
                })?
                .to_string();

            Ok(ActionEffects::Gate { command })
        }
    }
}

/// Build an agent notification effect for a standalone agent run.
pub fn build_agent_run_notify_effect(
    agent_run: &oj_core::AgentRun,
    agent_def: &AgentDef,
    message_template: Option<&String>,
) -> Option<Effect> {
    let template = message_template?;
    let mut vars: HashMap<String, String> = agent_run
        .vars
        .iter()
        .map(|(k, v)| (format!("var.{}", k), v.clone()))
        .collect();
    vars.insert("agent_run_id".to_string(), agent_run.id.clone());
    vars.insert("name".to_string(), agent_run.command_name.clone());
    vars.insert("agent".to_string(), agent_def.name.clone());
    if let Some(err) = &agent_run.error {
        vars.insert("error".to_string(), err.clone());
    }

    let message = oj_runbook::NotifyConfig::render(template, &vars);
    Some(Effect::Notify {
        title: agent_def.name.clone(),
        message,
    })
}

/// Build an agent notification effect if a message template is configured.
pub fn build_agent_notify_effect(
    job: &Job,
    agent_def: &AgentDef,
    message_template: Option<&String>,
) -> Option<Effect> {
    let template = message_template?;
    let mut vars: HashMap<String, String> = job
        .vars
        .iter()
        .map(|(k, v)| (format!("var.{}", k), v.clone()))
        .collect();
    vars.insert("job_id".to_string(), job.id.clone());
    vars.insert("name".to_string(), job.name.clone());
    vars.insert("agent".to_string(), agent_def.name.clone());
    vars.insert("step".to_string(), job.step.clone());
    if let Some(err) = &job.error {
        vars.insert("error".to_string(), err.clone());
    }

    let message = oj_runbook::NotifyConfig::render(template, &vars);
    Some(Effect::Notify {
        title: agent_def.name.clone(),
        message,
    })
}

/// Results from building action effects
#[derive(Debug)]
pub enum ActionEffects {
    /// Send nudge message to session
    Nudge { effects: Vec<Effect> },
    /// Advance to next job step
    AdvanceJob,
    /// Fail the job with an error
    FailJob { error: String },
    /// Resume by re-spawning agent with --resume (keeps workspace, preserves conversation)
    Resume {
        kill_session: Option<String>,
        agent_name: String,
        input: HashMap<String, String>,
        /// Claude --session-id from the previous run (for --resume).
        /// `None` when the prompt is being fully replaced (no `--resume`).
        resume_session_id: Option<String>,
    },
    /// Escalate to human
    Escalate { effects: Vec<Effect> },
    /// Run a shell gate command; advance if it passes, escalate if it fails
    Gate { command: String },
    /// Complete a standalone agent run
    CompleteAgentRun,
    /// Fail a standalone agent run
    FailAgentRun { error: String },
    /// Escalate a standalone agent run to human
    EscalateAgentRun { effects: Vec<Effect> },
}

#[cfg(test)]
#[path = "monitor_tests.rs"]
mod tests;
