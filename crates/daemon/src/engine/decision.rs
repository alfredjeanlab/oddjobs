// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Builder for escalation decisions.
//!
//! Creates DecisionCreated events with system-generated options
//! when escalation paths are triggered.

use oj_core::{AgentId, DecisionId, DecisionOption, DecisionSource, Event, OwnerId, QuestionData};
use std::time::{SystemTime, UNIX_EPOCH};

/// Trigger that caused the escalation.
#[derive(Debug, Clone)]
pub enum EscalationTrigger {
    /// Agent was idle for too long (on_idle)
    Idle { last_message: Option<String> },
    /// Agent process died unexpectedly (on_dead)
    Dead { exit_code: Option<i32>, last_message: Option<String> },
    /// Agent encountered an API/runtime error (on_error)
    Error { error_type: String, message: String, last_message: Option<String> },
    /// Gate command failed (gate action)
    GateFailed { command: String, exit_code: i32, stderr: String },
    /// Agent showed a permission prompt we couldn't handle (on_prompt)
    Prompt { prompt_type: String, last_message: Option<String> },
    /// Agent called AskUserQuestion — carries the parsed question data
    Question { questions: Option<QuestionData>, last_message: Option<String> },
    /// Agent called ExitPlanMode — carries the plan content
    Plan { last_message: Option<String> },
}

impl EscalationTrigger {
    pub fn to_source(&self) -> DecisionSource {
        match self {
            EscalationTrigger::Idle { .. } => DecisionSource::Idle,
            EscalationTrigger::Dead { .. } => DecisionSource::Dead,
            EscalationTrigger::Error { .. } => DecisionSource::Error,
            EscalationTrigger::GateFailed { .. } => DecisionSource::Gate,
            EscalationTrigger::Prompt { .. } => DecisionSource::Approval,
            EscalationTrigger::Question { .. } => DecisionSource::Question,
            EscalationTrigger::Plan { .. } => DecisionSource::Plan,
        }
    }
}

/// Build a DecisionCreated event for an escalation.
pub struct EscalationDecisionBuilder {
    /// Owner of the decision (job or crew)
    owner: OwnerId,
    /// Display name for context messages (job name or command name)
    display_name: String,
    /// Agent spawn UUID for the agent that triggered this decision.
    agent_id: String,
    trigger: EscalationTrigger,
    agent_log_tail: Option<String>,
    project: String,
}

impl EscalationDecisionBuilder {
    /// Create a decision builder for a job or crew.
    pub fn new(
        owner: OwnerId,
        display_name: String,
        agent_id: String,
        trigger: EscalationTrigger,
    ) -> Self {
        Self {
            owner,
            display_name,
            agent_id,
            trigger,
            agent_log_tail: None,
            project: String::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn agent_log_tail(mut self, tail: impl Into<String>) -> Self {
        self.agent_log_tail = Some(tail.into());
        self
    }

    pub fn project(mut self, ns: impl Into<String>) -> Self {
        self.project = ns.into();
        self
    }

    /// Build the DecisionCreated event and generated decision ID.
    pub fn build(self) -> (DecisionId, Event) {
        let decision_id = DecisionId::new();
        let context = self.build_context();
        let options = self.build_options();
        let created_at_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;

        // Extract questions from Question triggers
        let questions = match &self.trigger {
            EscalationTrigger::Question { questions, .. } => questions.clone(),
            _ => None,
        };

        let event = Event::DecisionCreated {
            id: decision_id,
            agent_id: AgentId::from_string(self.agent_id),
            owner: self.owner,
            source: self.trigger.to_source(),
            context,
            options,
            questions,
            created_at_ms,
            project: self.project,
        };

        (decision_id, event)
    }

    fn build_context(&self) -> String {
        let mut parts = Vec::new();

        // Trigger-specific header and extract last_message
        let last_message = match &self.trigger {
            EscalationTrigger::Idle { last_message, .. } => {
                parts.push(format!(
                    "Agent in job \"{}\" is idle and waiting for input.",
                    self.display_name
                ));
                last_message.as_deref()
            }
            EscalationTrigger::Dead { exit_code, last_message } => {
                let code_str = exit_code.map(|c| format!(" (exit code {})", c)).unwrap_or_default();
                parts.push(format!(
                    "Agent in job \"{}\" exited unexpectedly{}.",
                    self.display_name, code_str
                ));
                last_message.as_deref()
            }
            EscalationTrigger::Error { error_type, message, last_message } => {
                parts.push(format!(
                    "Agent in job \"{}\" encountered an error: {} - {}",
                    self.display_name, error_type, message
                ));
                last_message.as_deref()
            }
            EscalationTrigger::GateFailed { command, exit_code, stderr } => {
                parts.push(format!("Gate command failed in job \"{}\".", self.display_name));
                parts.push(format!("Command: {}", command));
                parts.push(format!("Exit code: {}", exit_code));
                if !stderr.is_empty() {
                    parts.push(format!("stderr:\n{}", stderr));
                }
                None
            }
            EscalationTrigger::Prompt { prompt_type, last_message } => {
                parts.push(format!(
                    "Agent in job \"{}\" is showing a {} prompt.",
                    self.display_name, prompt_type
                ));
                last_message.as_deref()
            }
            EscalationTrigger::Question { ref questions, last_message } => {
                if let Some(qd) = questions {
                    if let Some(entry) = qd.questions.first() {
                        let header = entry.header.as_deref().unwrap_or("Question");
                        parts.push(format!(
                            "Agent in job \"{}\" is asking a question.",
                            self.display_name
                        ));
                        parts.push(String::new());
                        parts.push(format!("[{}] {}", header, entry.question));

                        for q in qd.questions.iter().skip(1) {
                            let h = q.header.as_deref().unwrap_or("Question");
                            parts.push(format!("[{}] {}", h, q.question));
                        }
                    } else {
                        parts.push(format!(
                            "Agent in job \"{}\" is asking a question.",
                            self.display_name
                        ));
                    }
                } else {
                    parts.push(format!(
                        "Agent in job \"{}\" is asking a question (no details available).",
                        self.display_name
                    ));
                }
                last_message.as_deref()
            }
            EscalationTrigger::Plan { last_message } => {
                parts.push(format!(
                    "Agent in job \"{}\" is requesting plan approval.",
                    self.display_name
                ));
                last_message.as_deref()
            }
        };

        // Assistant context from session transcript
        if let Some(ctx) = last_message {
            if !ctx.is_empty() {
                // Truncate to ~2000 chars
                let truncated = if ctx.len() > 2000 { &ctx[..2000] } else { ctx };
                let label = match &self.trigger {
                    EscalationTrigger::Plan { .. } => "Plan",
                    EscalationTrigger::Question { .. } | EscalationTrigger::Prompt { .. } => {
                        "Agent Context"
                    }
                    _ => "Last Agent Message",
                };
                parts.push(format!("\n--- {} ---\n{}", label, truncated));
            }
        }

        // Agent log tail if available
        if let Some(tail) = &self.agent_log_tail {
            if !tail.is_empty() {
                parts.push(format!("\nRecent agent output:\n{}", tail));
            }
        }

        parts.join("\n")
    }

    fn build_options(&self) -> Vec<DecisionOption> {
        match &self.trigger {
            EscalationTrigger::Idle { .. } => vec![
                DecisionOption::new("Nudge")
                    .description("Send a message prompting the agent to continue")
                    .recommended(),
                DecisionOption::new("Done").description("Mark as complete"),
                DecisionOption::new("Cancel").description("Cancel and fail"),
                DecisionOption::new("Dismiss")
                    .description("Dismiss this notification without taking action"),
            ],
            EscalationTrigger::Dead { .. } | EscalationTrigger::Error { .. } => vec![
                DecisionOption::new("Retry")
                    .description("Restart the agent with --resume to continue")
                    .recommended(),
                DecisionOption::new("Skip").description("Skip and mark as complete"),
                DecisionOption::new("Cancel").description("Cancel and fail"),
                DecisionOption::new("Dismiss")
                    .description("Dismiss this notification without taking action"),
            ],
            EscalationTrigger::GateFailed { .. } => vec![
                DecisionOption::new("Retry").description("Re-run the gate command").recommended(),
                DecisionOption::new("Skip").description("Skip the gate and continue"),
                DecisionOption::new("Cancel").description("Cancel and fail"),
            ],
            EscalationTrigger::Prompt { .. } => vec![
                DecisionOption::new("Approve").description("Approve the pending action"),
                DecisionOption::new("Deny").description("Deny the pending action"),
                DecisionOption::new("Cancel").description("Cancel and fail"),
                DecisionOption::new("Dismiss")
                    .description("Dismiss this notification without taking action"),
            ],
            EscalationTrigger::Question { ref questions, .. } => {
                let mut options = Vec::new();

                if let Some(qd) = questions {
                    for entry in &qd.questions {
                        for opt in &entry.options {
                            let mut o = DecisionOption::new(opt.label.clone());
                            if let Some(ref desc) = opt.description {
                                o = o.description(desc.clone());
                            }
                            options.push(o);
                        }
                    }
                }

                // Always add Other, Cancel, and Dismiss as the last options
                options.push(DecisionOption::new("Other").description("Write a custom response"));
                options.push(DecisionOption::new("Cancel").description("Cancel and fail"));
                options.push(
                    DecisionOption::new("Dismiss")
                        .description("Dismiss this notification without taking action"),
                );

                options
            }
            EscalationTrigger::Plan { .. } => vec![
                DecisionOption::new("Accept (clear context)")
                    .description("Approve and auto-accept edits, clearing context")
                    .recommended(),
                DecisionOption::new("Accept (auto edits)")
                    .description("Approve and auto-accept edits"),
                DecisionOption::new("Accept (manual edits)")
                    .description("Approve with manual edit approval"),
                DecisionOption::new("Revise").description("Send feedback for plan revision"),
                DecisionOption::new("Cancel").description("Cancel and fail"),
            ],
        }
    }
}

#[cfg(test)]
#[path = "decision_tests.rs"]
mod decision_tests;
