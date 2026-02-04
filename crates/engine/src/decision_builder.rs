// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Builder for escalation decisions.
//!
//! Creates DecisionCreated events with system-generated options
//! when escalation paths are triggered.

use oj_core::{DecisionOption, DecisionSource, Event, PipelineId};
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
}

impl EscalationTrigger {
    pub fn to_source(&self) -> DecisionSource {
        match self {
            EscalationTrigger::Idle => DecisionSource::Idle,
            EscalationTrigger::Dead { .. } => DecisionSource::Error,
            EscalationTrigger::Error { .. } => DecisionSource::Error,
            EscalationTrigger::GateFailed { .. } => DecisionSource::Gate,
            EscalationTrigger::Prompt { .. } => DecisionSource::Approval,
        }
    }
}

/// Build a DecisionCreated event for an escalation.
pub struct EscalationDecisionBuilder {
    pipeline_id: PipelineId,
    pipeline_name: String,
    agent_id: Option<String>,
    trigger: EscalationTrigger,
    agent_log_tail: Option<String>,
    namespace: String,
}

impl EscalationDecisionBuilder {
    pub fn new(pipeline_id: PipelineId, pipeline_name: String, trigger: EscalationTrigger) -> Self {
        Self {
            pipeline_id,
            pipeline_name,
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

    pub fn agent_log_tail(mut self, tail: impl Into<String>) -> Self {
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

        let event = Event::DecisionCreated {
            id: decision_id.clone(),
            pipeline_id: self.pipeline_id,
            agent_id: self.agent_id,
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
                    "Agent in pipeline \"{}\" is idle and waiting for input.",
                    self.pipeline_name
                ));
            }
            EscalationTrigger::Dead { exit_code } => {
                let code_str = exit_code
                    .map(|c| format!(" (exit code {})", c))
                    .unwrap_or_default();
                parts.push(format!(
                    "Agent in pipeline \"{}\" exited unexpectedly{}.",
                    self.pipeline_name, code_str
                ));
            }
            EscalationTrigger::Error {
                error_type,
                message,
            } => {
                parts.push(format!(
                    "Agent in pipeline \"{}\" encountered an error: {} - {}",
                    self.pipeline_name, error_type, message
                ));
            }
            EscalationTrigger::GateFailed {
                command,
                exit_code,
                stderr,
            } => {
                parts.push(format!(
                    "Gate command failed in pipeline \"{}\".",
                    self.pipeline_name
                ));
                parts.push(format!("Command: {}", command));
                parts.push(format!("Exit code: {}", exit_code));
                if !stderr.is_empty() {
                    parts.push(format!("stderr:\n{}", stderr));
                }
            }
            EscalationTrigger::Prompt { prompt_type } => {
                parts.push(format!(
                    "Agent in pipeline \"{}\" is showing a {} prompt.",
                    self.pipeline_name, prompt_type
                ));
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
                    description: Some("Mark as complete and advance the pipeline".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel the pipeline".to_string()),
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
                    description: Some("Skip this step and advance the pipeline".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel the pipeline".to_string()),
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
                    description: Some("Skip the gate and advance the pipeline".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel the pipeline".to_string()),
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
                    description: Some("Cancel the pipeline".to_string()),
                    recommended: false,
                },
            ],
        }
    }
}

#[cfg(test)]
#[path = "decision_builder_tests.rs"]
mod tests;
