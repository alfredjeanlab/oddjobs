// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session monitoring for agent pipelines.
//!
//! Handles detection of agent state from session logs and triggers
//! appropriate actions (nudge, recover, escalate, etc.).

use crate::RuntimeError;
use oj_core::{
    AgentError, AgentState, Effect, Event, Pipeline, PipelineId, PromptType, SessionId, TimerId,
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
    Prompting { prompt_type: PromptType },
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

/// Get the current agent definition for a pipeline step
pub fn get_agent_def<'a>(
    runbook: &'a Runbook,
    pipeline: &Pipeline,
) -> Result<&'a AgentDef, RuntimeError> {
    let pipeline_def = runbook
        .get_pipeline(&pipeline.kind)
        .ok_or_else(|| RuntimeError::PipelineDefNotFound(pipeline.kind.clone()))?;

    let step_def = pipeline_def.get_step(&pipeline.step).ok_or_else(|| {
        RuntimeError::PipelineNotFound(format!("step {} not found", pipeline.step))
    })?;

    // Extract agent name from run directive
    let agent_name = match &step_def.run {
        RunDirective::Agent { agent, .. } => agent,
        _ => {
            return Err(RuntimeError::InvalidRunDirective {
                context: format!("step {}", pipeline.step),
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
    pipeline: &Pipeline,
    agent_def: &AgentDef,
    action_config: &ActionConfig,
    trigger: &str,
    input: &HashMap<String, String>,
) -> Result<ActionEffects, RuntimeError> {
    let action = action_config.action();
    let message = action_config.message();
    let pipeline_id = PipelineId::new(&pipeline.id);

    tracing::info!(
        pipeline_id = %pipeline.id,
        trigger = trigger,
        action = ?action,
        "building agent action effects"
    );

    match action {
        AgentAction::Nudge => {
            let session_id = pipeline
                .session_id
                .as_ref()
                .ok_or_else(|| RuntimeError::PipelineNotFound("no session".into()))?;

            let nudge_message = message.unwrap_or("Please continue with the task.");
            Ok(ActionEffects::Nudge {
                effects: vec![Effect::SendToSession {
                    session_id: SessionId::new(session_id),
                    input: format!("{}\n", nudge_message),
                }],
            })
        }

        AgentAction::Done => Ok(ActionEffects::AdvancePipeline),

        AgentAction::Fail => Ok(ActionEffects::FailPipeline {
            error: trigger.to_string(),
        }),

        AgentAction::Recover => {
            // Build modified input for re-spawn
            let mut new_inputs = input.clone();
            if let Some(msg) = message {
                if action_config.append() {
                    let existing = new_inputs.get("prompt").cloned().unwrap_or_default();
                    new_inputs.insert("prompt".to_string(), format!("{}\n\n{}", existing, msg));
                } else {
                    new_inputs.insert("prompt".to_string(), msg.to_string());
                }
            }

            Ok(ActionEffects::Recover {
                kill_session: pipeline.session_id.clone(),
                agent_name: agent_def.name.clone(),
                input: new_inputs,
            })
        }

        AgentAction::Escalate => {
            tracing::warn!(
                pipeline_id = %pipeline.id,
                trigger = trigger,
                message = ?message,
                "escalating to human"
            );

            let effects = vec![
                // Desktop notification
                Effect::Notify {
                    title: format!("Pipeline needs attention: {}", pipeline.name),
                    message: trigger.to_string(),
                },
                // Update pipeline status to Waiting
                Effect::Emit {
                    event: Event::StepWaiting {
                        pipeline_id: pipeline_id.clone(),
                        step: pipeline.step.clone(),
                        reason: None,
                        decision_id: None,
                    },
                },
                // Cancel exit-deferred timer (agent is still alive; liveness continues)
                Effect::CancelTimer {
                    id: TimerId::exit_deferred(&pipeline_id),
                },
            ];

            Ok(ActionEffects::Escalate { effects })
        }

        AgentAction::Gate => {
            let command = action_config
                .run()
                .ok_or_else(|| RuntimeError::InvalidRunDirective {
                    context: format!("pipeline {}", pipeline.id),
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

        AgentAction::Recover => {
            let mut new_inputs = input.clone();
            if let Some(msg) = message {
                if action_config.append() {
                    let existing = new_inputs.get("prompt").cloned().unwrap_or_default();
                    new_inputs.insert("prompt".to_string(), format!("{}\n\n{}", existing, msg));
                } else {
                    new_inputs.insert("prompt".to_string(), msg.to_string());
                }
            }

            Ok(ActionEffects::Recover {
                kill_session: agent_run.session_id.clone(),
                agent_name: agent_def.name.clone(),
                input: new_inputs,
            })
        }

        AgentAction::Escalate => {
            tracing::warn!(
                agent_run_id = %agent_run.id,
                trigger = trigger,
                message = ?message,
                "escalating standalone agent to human"
            );

            let effects = vec![
                Effect::Notify {
                    title: format!("Agent needs attention: {}", agent_run.command_name),
                    message: trigger.to_string(),
                },
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
    pipeline: &Pipeline,
    agent_def: &AgentDef,
    message_template: Option<&String>,
) -> Option<Effect> {
    let template = message_template?;
    let mut vars: HashMap<String, String> = pipeline
        .vars
        .iter()
        .map(|(k, v)| (format!("var.{}", k), v.clone()))
        .collect();
    vars.insert("pipeline_id".to_string(), pipeline.id.clone());
    vars.insert("name".to_string(), pipeline.name.clone());
    vars.insert("agent".to_string(), agent_def.name.clone());
    vars.insert("step".to_string(), pipeline.step.clone());
    if let Some(err) = &pipeline.error {
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
    /// Advance to next pipeline step
    AdvancePipeline,
    /// Fail the pipeline with an error
    FailPipeline { error: String },
    /// Recover by re-spawning agent (keeps workspace)
    Recover {
        kill_session: Option<String>,
        agent_name: String,
        input: HashMap<String, String>,
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
