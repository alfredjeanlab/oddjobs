// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Builder for escalation decisions.
//!
//! Creates DecisionCreated events with system-generated options
//! when escalation paths are triggered.

use oj_core::{AgentRunId, DecisionOption, DecisionSource, Event, JobId, OwnerId, QuestionData};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Trigger that caused the escalation.
#[derive(Debug, Clone)]
pub enum EscalationTrigger {
    /// Agent was idle for too long (on_idle)
    Idle { assistant_context: Option<String> },
    /// Agent process died unexpectedly (on_dead)
    Dead {
        exit_code: Option<i32>,
        assistant_context: Option<String>,
    },
    /// Agent encountered an API/runtime error (on_error)
    Error {
        error_type: String,
        message: String,
        assistant_context: Option<String>,
    },
    /// Gate command failed (gate action)
    GateFailed {
        command: String,
        exit_code: i32,
        stderr: String,
    },
    /// Agent showed a permission prompt we couldn't handle (on_prompt)
    Prompt {
        prompt_type: String,
        assistant_context: Option<String>,
    },
    /// Agent called AskUserQuestion — carries the parsed question data
    Question {
        question_data: Option<QuestionData>,
        assistant_context: Option<String>,
    },
    /// Agent called ExitPlanMode — carries the plan content
    Plan { assistant_context: Option<String> },
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
    /// Owner of the decision (job or agent_run)
    owner: OwnerId,
    /// Display name for context messages (job name or command name)
    display_name: String,
    agent_id: Option<String>,
    trigger: EscalationTrigger,
    agent_log_tail: Option<String>,
    namespace: String,
}

impl EscalationDecisionBuilder {
    /// Create a decision builder for a job.
    pub fn for_job(job_id: JobId, job_name: String, trigger: EscalationTrigger) -> Self {
        Self {
            owner: OwnerId::Job(job_id),
            display_name: job_name,
            agent_id: None,
            trigger,
            agent_log_tail: None,
            namespace: String::new(),
        }
    }

    /// Create a decision builder for a standalone agent run.
    pub fn for_agent_run(
        agent_run_id: AgentRunId,
        command_name: String,
        trigger: EscalationTrigger,
    ) -> Self {
        Self {
            owner: OwnerId::AgentRun(agent_run_id),
            display_name: command_name,
            agent_id: None,
            trigger,
            agent_log_tail: None,
            namespace: String::new(),
        }
    }

    pub fn agent_id(mut self, id: impl Into<String>) -> Self {
        self.agent_id = Some(id.into());
        self
    }

    #[cfg(test)]
    pub(crate) fn agent_log_tail(mut self, tail: impl Into<String>) -> Self {
        self.agent_log_tail = Some(tail.into());
        self
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = ns.into();
        self
    }

    /// Build the DecisionCreated event and generated decision ID.
    pub fn build(self) -> (String, Event) {
        let decision_id = Uuid::new_v4().to_string();
        let context = self.build_context();
        let options = self.build_options();
        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Extract job_id from owner (empty for agent runs)
        let job_id = match &self.owner {
            OwnerId::Job(id) => id.clone(),
            OwnerId::AgentRun(_) => JobId::default(),
        };

        // Extract question_data from Question triggers
        let question_data = match &self.trigger {
            EscalationTrigger::Question { question_data, .. } => question_data.clone(),
            _ => None,
        };

        let event = Event::DecisionCreated {
            id: decision_id.clone(),
            job_id,
            agent_id: self.agent_id,
            owner: self.owner,
            source: self.trigger.to_source(),
            context,
            options,
            question_data,
            created_at_ms,
            namespace: self.namespace,
        };

        (decision_id, event)
    }

    fn build_context(&self) -> String {
        let mut parts = Vec::new();

        // Trigger-specific header and extract assistant_context
        let assistant_context = match &self.trigger {
            EscalationTrigger::Idle {
                assistant_context, ..
            } => {
                parts.push(format!(
                    "Agent in job \"{}\" is idle and waiting for input.",
                    self.display_name
                ));
                assistant_context.as_deref()
            }
            EscalationTrigger::Dead {
                exit_code,
                assistant_context,
            } => {
                let code_str = exit_code
                    .map(|c| format!(" (exit code {})", c))
                    .unwrap_or_default();
                parts.push(format!(
                    "Agent in job \"{}\" exited unexpectedly{}.",
                    self.display_name, code_str
                ));
                assistant_context.as_deref()
            }
            EscalationTrigger::Error {
                error_type,
                message,
                assistant_context,
            } => {
                parts.push(format!(
                    "Agent in job \"{}\" encountered an error: {} - {}",
                    self.display_name, error_type, message
                ));
                assistant_context.as_deref()
            }
            EscalationTrigger::GateFailed {
                command,
                exit_code,
                stderr,
            } => {
                parts.push(format!(
                    "Gate command failed in job \"{}\".",
                    self.display_name
                ));
                parts.push(format!("Command: {}", command));
                parts.push(format!("Exit code: {}", exit_code));
                if !stderr.is_empty() {
                    parts.push(format!("stderr:\n{}", stderr));
                }
                None
            }
            EscalationTrigger::Prompt {
                prompt_type,
                assistant_context,
            } => {
                parts.push(format!(
                    "Agent in job \"{}\" is showing a {} prompt.",
                    self.display_name, prompt_type
                ));
                assistant_context.as_deref()
            }
            EscalationTrigger::Question {
                ref question_data,
                assistant_context,
            } => {
                if let Some(qd) = question_data {
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
                assistant_context.as_deref()
            }
            EscalationTrigger::Plan { assistant_context } => {
                parts.push(format!(
                    "Agent in job \"{}\" is requesting plan approval.",
                    self.display_name
                ));
                assistant_context.as_deref()
            }
        };

        // Assistant context from session transcript
        if let Some(ctx) = assistant_context {
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
                DecisionOption::new("Retry")
                    .description("Re-run the gate command")
                    .recommended(),
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
            EscalationTrigger::Question {
                ref question_data, ..
            } => {
                let mut options = Vec::new();

                if let Some(qd) = question_data {
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

                // Always add Cancel and Dismiss as the last options
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
#[path = "decision_builder_tests.rs"]
mod tests;
