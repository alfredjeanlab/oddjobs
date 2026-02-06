// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision types for human-in-the-loop job control.

use crate::owner::OwnerId;
use serde::{Deserialize, Serialize};

crate::define_id! {
    /// Unique identifier for a decision.
    pub struct DecisionId;
}

/// Where the decision originated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    Question,
    Approval,
    Gate,
    Error,
    Idle,
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
    /// Job ID (kept for backward compatibility; empty for agent runs)
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Owner of this decision (job or agent_run).
    pub owner: OwnerId,
    pub source: DecisionSource,
    pub context: String,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    /// 1-indexed choice (None = unresolved or freeform-only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<usize>,
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
    #[serde(default)]
    pub namespace: String,
}

impl DecisionOption {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
            recommended: false,
        }
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

impl Decision {
    pub fn is_resolved(&self) -> bool {
        self.resolved_at_ms.is_some()
    }
}

#[cfg(test)]
#[path = "decision_tests.rs"]
mod tests;
