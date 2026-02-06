// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent event types and helpers

use super::Event;
use crate::agent::{AgentId, AgentState};
use crate::owner::OwnerId;
use serde::{Deserialize, Serialize};

/// Kind of signal an agent can emit
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSignalKind {
    /// Advance the job to the next step
    Complete,
    /// Pause the job and notify for human intervention
    Escalate,
    /// No-op acknowledgement — agent is still working
    Continue,
}

/// Type of prompt the agent is showing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptType {
    Permission,
    Idle,
    PlanApproval,
    Question,
    Other,
}

/// Structured data from an AskUserQuestion tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionData {
    pub questions: Vec<QuestionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionEntry {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default, rename = "multiSelect")]
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

pub(super) fn default_prompt_type() -> PromptType {
    PromptType::Other
}

/// Create an agent event from an AgentState with owner.
pub(super) fn from_agent_state(agent_id: AgentId, state: AgentState, owner: OwnerId) -> Event {
    match state {
        AgentState::Working => Event::AgentWorking { agent_id, owner },
        AgentState::WaitingForInput => Event::AgentWaiting { agent_id, owner },
        AgentState::Failed(error) => Event::AgentFailed {
            agent_id,
            error,
            owner,
        },
        AgentState::Exited { exit_code } => Event::AgentExited {
            agent_id,
            exit_code,
            owner,
        },
        AgentState::SessionGone => Event::AgentGone { agent_id, owner },
    }
}

/// Extract agent_id, state, and owner if this is an agent state event.
pub(super) fn as_agent_state(event: &Event) -> Option<(&AgentId, AgentState, &OwnerId)> {
    match event {
        Event::AgentWorking { agent_id, owner } => Some((agent_id, AgentState::Working, owner)),
        Event::AgentWaiting { agent_id, owner } => {
            Some((agent_id, AgentState::WaitingForInput, owner))
        }
        Event::AgentFailed {
            agent_id,
            error,
            owner,
        } => Some((agent_id, AgentState::Failed(error.clone()), owner)),
        Event::AgentExited {
            agent_id,
            exit_code,
            owner,
        } => Some((
            agent_id,
            AgentState::Exited {
                exit_code: *exit_code,
            },
            owner,
        )),
        Event::AgentGone { agent_id, owner } => Some((agent_id, AgentState::SessionGone, owner)),
        _ => None,
    }
}

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        Event::AgentWorking { agent_id, .. }
        | Event::AgentWaiting { agent_id, .. }
        | Event::AgentFailed { agent_id, .. }
        | Event::AgentExited { agent_id, .. }
        | Event::AgentGone { agent_id, .. } => format!("{t} agent={agent_id}"),
        Event::AgentInput { agent_id, .. } => format!("{t} agent={agent_id}"),
        Event::AgentSignal { agent_id, kind, .. } => {
            format!("{t} id={agent_id} kind={kind:?}")
        }
        Event::AgentIdle { agent_id } => format!("{t} agent={agent_id}"),
        Event::AgentStop { agent_id } => format!("{t} agent={agent_id}"),
        Event::AgentPrompt {
            agent_id,
            prompt_type,
            ..
        } => format!("{t} agent={agent_id} prompt_type={prompt_type:?}"),
        _ => unreachable!("not an agent event"),
    }
}
