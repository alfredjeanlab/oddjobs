// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent identifier and state types.
//!
//! AgentId is distinct from session_id (internal to adapters) and workspace_id
//! (the git worktree path). An agent represents a single invocation of an AI
//! agent within a pipeline step.

use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

/// Unique identifier for an agent instance.
///
/// Typically formatted as `{pipeline_id}-{step}` but the format is opaque
/// to consumers. Session IDs are hidden inside the AgentAdapter implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    /// Create a new AgentId from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the string value of this AgentId.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl PartialEq<str> for AgentId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AgentId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl Borrow<str> for AgentId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// State of an agent as detected from monitoring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    /// Agent is actively working (processing or running tools)
    Working,
    /// Agent finished and is waiting for user input
    WaitingForInput,
    /// Agent encountered a failure
    Failed(AgentError),
    /// Agent process exited
    Exited { exit_code: Option<i32> },
    /// Agent session is gone (process terminated unexpectedly)
    SessionGone,
}

/// Categorized failure reasons for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentError {
    /// Invalid API key or authentication failure
    Unauthorized,
    /// Exceeded quota or billing issue
    OutOfCredits,
    /// Network connectivity issue
    NoInternet,
    /// Rate limited by API
    RateLimited,
    /// Other error with message
    Other(String),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Unauthorized => write!(f, "unauthorized"),
            AgentError::OutOfCredits => write!(f, "out of credits"),
            AgentError::NoInternet => write!(f, "no internet connection"),
            AgentError::RateLimited => write!(f, "rate limited"),
            AgentError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentState::Working => write!(f, "working"),
            AgentState::WaitingForInput => write!(f, "waiting for input"),
            AgentState::Failed(reason) => write!(f, "failed: {}", reason),
            AgentState::Exited {
                exit_code: Some(code),
            } => write!(f, "exited with code {}", code),
            AgentState::Exited { exit_code: None } => write!(f, "exited"),
            AgentState::SessionGone => write!(f, "session gone"),
        }
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
