// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared action attempt tracking and agent signal state.
//!
//! Used by both `Job` and `AgentRun` to manage retry logic for
//! lifecycle actions (on_idle, on_dead, etc.).

use crate::event::AgentSignalKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Signal from agent indicating completion intent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSignal {
    pub kind: AgentSignalKind,
    pub message: Option<String>,
}

/// Tracks action attempt counts and agent signal state.
///
/// Embedded in both `Job` and `AgentRun` via `#[serde(flatten)]`
/// for backward-compatible serialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionTracker {
    /// Attempt counts per (trigger, chain_position).
    /// Key format: "trigger:chain_pos" (e.g., "on_fail:0").
    #[serde(default)]
    pub action_attempts: HashMap<String, u32>,
    /// Signal from agent indicating completion intent.
    /// Cleared on step/state transitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_signal: Option<AgentSignal>,
}

impl ActionTracker {
    /// Build the string key for action_attempts.
    fn action_key(trigger: &str, chain_pos: usize) -> String {
        format!("{trigger}:{chain_pos}")
    }

    /// Increment and return the new attempt count for a given action.
    pub fn increment_action_attempt(&mut self, trigger: &str, chain_pos: usize) -> u32 {
        let key = Self::action_key(trigger, chain_pos);
        let count = self.action_attempts.entry(key).or_insert(0);
        *count += 1;
        *count
    }

    /// Get current attempt count for a given action.
    pub fn get_action_attempt(&self, trigger: &str, chain_pos: usize) -> u32 {
        self.action_attempts
            .get(&Self::action_key(trigger, chain_pos))
            .copied()
            .unwrap_or(0)
    }

    /// Reset all action attempts.
    pub fn reset_action_attempts(&mut self) {
        self.action_attempts.clear();
    }

    /// Clear agent signal.
    pub fn clear_agent_signal(&mut self) {
        self.agent_signal = None;
    }
}

#[cfg(test)]
#[path = "action_tracker_tests.rs"]
mod tests;
