// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision types for human-in-the-loop job control.

use crate::event::QuestionData;
use crate::owner::OwnerId;
use crate::AgentId;
use serde::{Deserialize, Serialize};

crate::define_id! {
    /// Unique identifier for a decision.
    pub struct DecisionId("dcn-");
}

/// Where the decision originated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    Question,
    Approval,
    Gate,
    Error,
    Dead,
    Idle,
    Plan,
}

/// A single option the user can choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub recommended: bool,
}

/// A decision awaiting (or resolved by) human input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: DecisionId,
    pub agent_id: AgentId,
    pub owner: OwnerId,
    pub project: String,
    pub source: DecisionSource,
    pub context: String,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    /// Structured question data for multi-question decisions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questions: Option<QuestionData>,
    /// Per-question 1-indexed answers for multi-question decisions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<usize>,
    /// Freeform message from the resolver
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_ms: Option<u64>,
    /// Set when this decision was auto-dismissed because a newer decision
    /// was created for the same owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<DecisionId>,
}

impl DecisionOption {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), description: None, recommended: false }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn recommended(mut self) -> Self {
        self.recommended = true;
        self
    }
}

impl DecisionSource {
    /// Whether a new decision with this source should supersede an existing one.
    ///
    /// Prevents less-specific prompt types from overriding more-specific ones.
    /// For example, a `permission_prompt` notification (Approval) should not
    /// supersede an AskUserQuestion decision (Question) that was created by
    /// the more-specific PreToolUse hook.
    /// Whether this decision was created while the agent was alive.
    ///
    /// Alive decisions become stale when the agent dies and should be
    /// auto-dismissed so on_dead can fire with appropriate options.
    pub fn is_alive_agent_source(&self) -> bool {
        matches!(self, Self::Idle | Self::Question | Self::Plan | Self::Approval)
    }

    pub fn should_supersede(&self, existing: &DecisionSource) -> bool {
        match (self, existing) {
            // Approval (generic permission prompt) cannot supersede Question or Plan
            (DecisionSource::Approval, DecisionSource::Question) => false,
            (DecisionSource::Approval, DecisionSource::Plan) => false,
            _ => true,
        }
    }
}

impl Decision {
    pub fn is_resolved(&self) -> bool {
        self.resolved_at_ms.is_some()
    }

    pub fn chosen(&self) -> Option<usize> {
        self.choices.first().copied()
    }
}

#[cfg(test)]
#[path = "decision_tests.rs"]
mod tests;
