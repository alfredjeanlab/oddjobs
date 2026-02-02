// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer identifier type for tracking scheduled timers.
//!
//! TimerId uniquely identifies a timer instance used for scheduling delayed
//! actions such as timeouts, heartbeats, or periodic checks.

use crate::pipeline::PipelineId;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

/// Unique identifier for a timer instance.
///
/// Timers are used to schedule delayed actions within the system, such as
/// step timeouts or periodic health checks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimerId(pub String);

impl TimerId {
    /// Create a new TimerId from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the string value of this TimerId.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Timer ID for liveness monitoring of a pipeline.
    pub fn liveness(pipeline_id: &PipelineId) -> Self {
        Self::new(format!("liveness:{}", pipeline_id))
    }

    /// Timer ID for deferred exit handling of a pipeline.
    pub fn exit_deferred(pipeline_id: &PipelineId) -> Self {
        Self::new(format!("exit-deferred:{}", pipeline_id))
    }

    /// Timer ID for cooldown between action attempts.
    pub fn cooldown(pipeline_id: &PipelineId, trigger: &str, chain_pos: usize) -> Self {
        Self::new(format!(
            "cooldown:{}:{}:{}",
            pipeline_id, trigger, chain_pos
        ))
    }

    /// Returns true if this is a liveness timer.
    pub fn is_liveness(&self) -> bool {
        self.0.starts_with("liveness:")
    }

    /// Returns true if this is an exit-deferred timer.
    pub fn is_exit_deferred(&self) -> bool {
        self.0.starts_with("exit-deferred:")
    }

    /// Returns true if this is a cooldown timer.
    pub fn is_cooldown(&self) -> bool {
        self.0.starts_with("cooldown:")
    }

    /// Timer ID for queue item retry cooldown.
    pub fn queue_retry(queue_name: &str, item_id: &str) -> Self {
        Self::new(format!("queue-retry:{}:{}", queue_name, item_id))
    }

    /// Returns true if this is a queue retry timer.
    pub fn is_queue_retry(&self) -> bool {
        self.0.starts_with("queue-retry:")
    }

    /// Extracts the pipeline ID portion if this is a pipeline-related timer.
    ///
    /// Returns `Some(&str)` for liveness, exit-deferred, and cooldown timers.
    /// For cooldown timers, extracts the pipeline_id from "cooldown:pipeline_id:trigger:pos".
    pub fn pipeline_id_str(&self) -> Option<&str> {
        if let Some(rest) = self.0.strip_prefix("liveness:") {
            Some(rest)
        } else if let Some(rest) = self.0.strip_prefix("exit-deferred:") {
            Some(rest)
        } else if let Some(rest) = self.0.strip_prefix("cooldown:") {
            // Format: "cooldown:pipeline_id:trigger:chain_pos"
            rest.split(':').next()
        } else {
            None
        }
    }
}

impl fmt::Display for TimerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for TimerId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for TimerId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl PartialEq<str> for TimerId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for TimerId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl Borrow<str> for TimerId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "timer_tests.rs"]
mod tests;
