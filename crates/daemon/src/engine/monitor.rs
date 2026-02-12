// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session monitoring for agent jobs.
//!
//! Handles detection of agent state from session logs and triggers
//! appropriate actions (nudge, resume, escalate, etc.).

use crate::engine::decision_builder::{EscalationDecisionBuilder, EscalationTrigger};
use crate::engine::lifecycle::RunLifecycle;
use crate::engine::RuntimeError;
use oj_core::{AgentError, AgentId, AgentState, Effect, Job, PromptType, QuestionData};
use oj_runbook::{ActionConfig, AgentAction, AgentDef, ErrorType, RunDirective, Runbook};
use std::collections::HashMap;
use std::time::Duration;

/// Bundled parameters for action execution (on_idle, on_dead, etc.).
pub(crate) struct ActionContext<'a> {
    pub agent_def: &'a AgentDef,
    pub action_config: &'a ActionConfig,
    pub trigger: &'a str,
    pub chain_pos: usize,
    pub questions: Option<&'a QuestionData>,
    pub last_message: Option<&'a str>,
}

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

    let num: u64 = num_str.parse().map_err(|_| format!("invalid number in duration: {}", s))?;

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
        questions: Option<QuestionData>,
        last_message: Option<String>,
    },
    /// Agent encountered an error
    Failed { message: String, error_type: Option<ErrorType> },
    /// Agent process exited
    Exited { exit_code: Option<i32> },
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
            AgentState::Exited { exit_code } => MonitorState::Exited { exit_code: *exit_code },
            AgentState::SessionGone => MonitorState::Gone,
        }
    }
}

/// Format exit message for agent exit logging.
pub(crate) fn format_exit_message(exit_code: Option<i32>) -> String {
    match exit_code {
        Some(code) => format!("agent exited (exit code: {})", code),
        None => "agent exited".to_string(),
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
    let job_def =
        runbook.get_job(&job.kind).ok_or_else(|| RuntimeError::JobDefNotFound(job.kind.clone()))?;

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

    runbook.get_agent(agent_name).ok_or_else(|| RuntimeError::AgentNotFound(agent_name.clone()))
}

/// Build effects for an agent action (nudge, recover, escalate, etc.)
///
/// Unified across Job and Crew via the `RunLifecycle` trait.
pub fn build_action_effects_for(
    ctx: &ActionContext<'_>,
    run: &dyn RunLifecycle,
) -> Result<ActionEffects, RuntimeError> {
    let action = ctx.action_config.action();
    let message = ctx.action_config.message();

    tracing::info!(
        run_id = %run.log_id(),
        trigger = ctx.trigger,
        action = ?action,
        "building agent action effects"
    );

    match action {
        AgentAction::Nudge => {
            let agent_id = run
                .agent_id()
                .ok_or_else(|| RuntimeError::InvalidRequest("no agent for nudge".into()))?;

            let nudge_message = message.unwrap_or("Please continue with the task.");
            Ok(ActionEffects::Nudge {
                effects: vec![Effect::SendToAgent {
                    agent_id: AgentId::new(agent_id),
                    input: format!("{}\n", nudge_message),
                }],
            })
        }

        AgentAction::Done => Ok(ActionEffects::Advance),

        AgentAction::Fail => Ok(ActionEffects::Fail { error: ctx.trigger.to_string() }),

        AgentAction::Resume => {
            let mut new_inputs = run.vars().clone();
            let use_resume = message.is_none() || ctx.action_config.append();

            if let Some(msg) = message {
                if ctx.action_config.append() && use_resume {
                    new_inputs.insert("resume_message".to_string(), msg.to_string());
                } else {
                    new_inputs.insert("prompt".to_string(), msg.to_string());
                }
            }

            // Resume when there was a previous agent (a session to resume)
            let resume = use_resume && run.agent_id().is_some();

            Ok(ActionEffects::Resume {
                kill_agent: run.agent_id().map(|s| s.to_string()),
                agent_name: ctx.agent_def.name.clone(),
                input: new_inputs,
                resume,
            })
        }

        AgentAction::Escalate => {
            tracing::warn!(
                run_id = %run.log_id(),
                trigger = ctx.trigger,
                message = ?message,
                "escalating to human — creating decision"
            );

            let ac = ctx.last_message.map(|s| s.to_string());
            let escalation_trigger = build_escalation_trigger(ctx.trigger, message, ac, ctx);

            let (decision_id, decision_event) = EscalationDecisionBuilder::new(
                run.owner_id(),
                run.display_name().to_string(),
                run.decision_agent_ref(),
                escalation_trigger,
            )
            .project(run.project())
            .build();

            let mut effects = vec![
                Effect::Emit { event: decision_event },
                Effect::Notify {
                    title: format!("Decision needed: {}", run.display_name()),
                    message: format!("Requires attention ({})", ctx.trigger),
                },
            ];

            effects.extend(run.escalation_status_effects(ctx.trigger, Some(decision_id.as_str())));

            Ok(ActionEffects::Escalate { effects })
        }

        AgentAction::Gate => {
            let command = ctx
                .action_config
                .run()
                .ok_or_else(|| RuntimeError::InvalidRunDirective {
                    context: run.log_id().to_string(),
                    directive: "gate action requires a 'run' field".to_string(),
                })?
                .to_string();

            Ok(ActionEffects::Gate { command })
        }

        AgentAction::Auto => {
            // Auto mode delegates to coop's self-determination UI.
            // The engine does not intervene.
            Ok(ActionEffects::Advance)
        }
    }
}

/// Map a trigger string to an EscalationTrigger.
fn build_escalation_trigger(
    trigger: &str,
    message: Option<&str>,
    ac: Option<String>,
    ctx: &ActionContext<'_>,
) -> EscalationTrigger {
    match trigger {
        "idle" | "on_idle" => EscalationTrigger::Idle { last_message: ac },
        "dead" | "on_dead" | "exit" | "exited" => {
            EscalationTrigger::Dead { exit_code: None, last_message: ac }
        }
        "error" | "on_error" => EscalationTrigger::Error {
            error_type: "unknown".to_string(),
            message: message.unwrap_or("").to_string(),
            last_message: ac,
        },
        "prompt:question" => {
            EscalationTrigger::Question { questions: ctx.questions.cloned(), last_message: ac }
        }
        "prompt:plan" => EscalationTrigger::Plan { last_message: ac },
        "prompt" | "on_prompt" => {
            EscalationTrigger::Prompt { prompt_type: "permission".to_string(), last_message: ac }
        }
        t if t.ends_with(":exhausted") => {
            let base = t.trim_end_matches(":exhausted");
            match base {
                "idle" => EscalationTrigger::Idle { last_message: ac },
                "error" => EscalationTrigger::Error {
                    error_type: "exhausted".to_string(),
                    message: message.unwrap_or("").to_string(),
                    last_message: ac,
                },
                _ => EscalationTrigger::Dead { exit_code: None, last_message: ac },
            }
        }
        _ => EscalationTrigger::Idle { last_message: ac }, // fallback
    }
}

/// Results from building action effects.
///
/// Unified across job and crew actions. The caller dispatches
/// `Advance`, `Fail`, and `Escalate` to the appropriate entity handler.
#[derive(Debug)]
pub enum ActionEffects {
    /// Send nudge message to session
    Nudge { effects: Vec<Effect> },
    /// Advance to next step (job) or complete (crew)
    Advance,
    /// Fail with an error
    Fail { error: String },
    /// Resume by re-spawning agent with --resume (keeps workspace, preserves conversation)
    Resume {
        kill_agent: Option<String>,
        agent_name: String,
        input: HashMap<String, String>,
        /// Whether to resume the previous session (coop handles discovery).
        resume: bool,
    },
    /// Escalate to human (effects include DecisionCreated, notifications, etc.)
    Escalate { effects: Vec<Effect> },
    /// Run a shell gate command; advance if it passes, escalate if it fails
    Gate { command: String },
}

#[cfg(test)]
#[path = "monitor_tests.rs"]
mod tests;
