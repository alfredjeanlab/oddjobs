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
    Idle,
    /// Agent process died unexpectedly (on_dead)
    Dead { exit_code: Option<i32> },
    /// Agent encountered an API/runtime error (on_error)
    Error { error_type: String, message: String },
    /// Gate command failed (gate action)
    GateFailed {
        command: String,
        exit_code: i32,
        stderr: String,
    },
    /// Agent showed a permission prompt we couldn't handle (on_prompt)
    Prompt { prompt_type: String },
    /// Agent called AskUserQuestion — carries the parsed question data
    Question { question_data: Option<QuestionData> },
}

impl EscalationTrigger {
    pub fn to_source(&self) -> DecisionSource {
        match self {
            EscalationTrigger::Idle => DecisionSource::Idle,
            EscalationTrigger::Dead { .. } => DecisionSource::Error,
            EscalationTrigger::Error { .. } => DecisionSource::Error,
            EscalationTrigger::GateFailed { .. } => DecisionSource::Gate,
            EscalationTrigger::Prompt { .. } => DecisionSource::Approval,
            EscalationTrigger::Question { .. } => DecisionSource::Question,
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

        let event = Event::DecisionCreated {
            id: decision_id.clone(),
            job_id,
            agent_id: self.agent_id,
            owner: self.owner,
            source: self.trigger.to_source(),
            context,
            options,
            created_at_ms,
            namespace: self.namespace,
        };

        (decision_id, event)
    }

    fn build_context(&self) -> String {
        let mut parts = Vec::new();

        // Trigger-specific header
        match &self.trigger {
            EscalationTrigger::Idle => {
                parts.push(format!(
                    "Agent in job \"{}\" is idle and waiting for input.",
                    self.display_name
                ));
            }
            EscalationTrigger::Dead { exit_code } => {
                let code_str = exit_code
                    .map(|c| format!(" (exit code {})", c))
                    .unwrap_or_default();
                parts.push(format!(
                    "Agent in job \"{}\" exited unexpectedly{}.",
                    self.display_name, code_str
                ));
            }
            EscalationTrigger::Error {
                error_type,
                message,
            } => {
                parts.push(format!(
                    "Agent in job \"{}\" encountered an error: {} - {}",
                    self.display_name, error_type, message
                ));
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
            }
            EscalationTrigger::Prompt { prompt_type } => {
                parts.push(format!(
                    "Agent in job \"{}\" is showing a {} prompt.",
                    self.display_name, prompt_type
                ));
            }
            EscalationTrigger::Question { ref question_data } => {
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
            EscalationTrigger::Idle => vec![
                DecisionOption {
                    label: "Nudge".to_string(),
                    description: Some("Send a message prompting the agent to continue".to_string()),
                    recommended: true,
                },
                DecisionOption {
                    label: "Done".to_string(),
                    description: Some("Mark as complete".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel and fail".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Dismiss".to_string(),
                    description: Some(
                        "Dismiss this notification without taking action".to_string(),
                    ),
                    recommended: false,
                },
            ],
            EscalationTrigger::Dead { .. } | EscalationTrigger::Error { .. } => vec![
                DecisionOption {
                    label: "Retry".to_string(),
                    description: Some("Restart the agent with --resume to continue".to_string()),
                    recommended: true,
                },
                DecisionOption {
                    label: "Skip".to_string(),
                    description: Some("Skip and mark as complete".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel and fail".to_string()),
                    recommended: false,
                },
            ],
            EscalationTrigger::GateFailed { .. } => vec![
                DecisionOption {
                    label: "Retry".to_string(),
                    description: Some("Re-run the gate command".to_string()),
                    recommended: true,
                },
                DecisionOption {
                    label: "Skip".to_string(),
                    description: Some("Skip the gate and continue".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel and fail".to_string()),
                    recommended: false,
                },
            ],
            EscalationTrigger::Prompt { .. } => vec![
                DecisionOption {
                    label: "Approve".to_string(),
                    description: Some("Approve the pending action".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Deny".to_string(),
                    description: Some("Deny the pending action".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel and fail".to_string()),
                    recommended: false,
                },
            ],
            EscalationTrigger::Question { ref question_data } => {
                let mut options = Vec::new();

                if let Some(qd) = question_data {
                    if let Some(entry) = qd.questions.first() {
                        for opt in &entry.options {
                            options.push(DecisionOption {
                                label: opt.label.clone(),
                                description: opt.description.clone(),
                                recommended: false,
                            });
                        }
                    }
                }

                // Always add Cancel as the last option
                options.push(DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel and fail".to_string()),
                    recommended: false,
                });

                options
            }
        }
    }
}

#[cfg(test)]
#[path = "decision_builder_tests.rs"]
mod tests;
