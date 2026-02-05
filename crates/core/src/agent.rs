// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent identifier and state types.
//!
//! AgentId is distinct from session_id (internal to adapters) and workspace_id
//! (the git worktree path). An agent represents a single invocation of an AI
//! agent within a job step.

use serde::{Deserialize, Serialize};
use std::fmt;

crate::define_id! {
    /// Unique identifier for an agent instance.
    ///
    /// Typically formatted as `{job_id}-{step}` but the format is opaque
    /// to consumers. Session IDs are hidden inside the AgentAdapter implementation.
    pub struct AgentId;
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
