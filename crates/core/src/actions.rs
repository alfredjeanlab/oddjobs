// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared action attempt tracking.
//!
//! Used by both `Job` and `Crew` to manage retry logic for
//! lifecycle actions (on_idle, on_dead, etc.).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tracks action attempt counts.
///
/// Embedded in both `Job` and `Crew` via `#[serde(flatten)]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionTracker {
    /// Attempt counts per (trigger, chain_position).
    /// Key format: "trigger:chain_pos" (e.g., "on_fail:0").
    #[serde(default)]
    pub attempts: HashMap<String, u32>,
}

impl ActionTracker {
    /// Build the string key for attempts.
    fn action_key(trigger: &str, chain_pos: usize) -> String {
        format!("{trigger}:{chain_pos}")
    }

    /// Increment and return the new attempt count for a given action.
    pub fn increment_attempt(&mut self, trigger: &str, chain_pos: usize) -> u32 {
        let key = Self::action_key(trigger, chain_pos);
        let count = self.attempts.entry(key).or_insert(0);
        *count += 1;
        *count
    }

    /// Get current attempt count for a given action.
    pub fn get_action_attempt(&self, trigger: &str, chain_pos: usize) -> u32 {
        self.attempts.get(&Self::action_key(trigger, chain_pos)).copied().unwrap_or(0)
    }

    /// Reset all action attempts.
    pub fn reset_attempts(&mut self) {
        self.attempts.clear();
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
