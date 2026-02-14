// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent identifier and state types.
//!
//! AgentId is distinct from workspace_id (the git worktree path). An agent
//! represents a single invocation of an AI agent within a job step.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

crate::define_id! {
    /// Unique identifier for an agent instance.
    ///
    /// Typically formatted as `{job_id}-{step}` but the format is opaque
    /// to consumers. Session IDs are hidden inside the AgentAdapter implementation.
    pub struct AgentId("agt-");
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
            AgentState::Exited { exit_code: Some(code) } => write!(f, "exited with code {}", code),
            AgentState::Exited { exit_code: None } => write!(f, "exited"),
            AgentState::SessionGone => write!(f, "session gone"),
        }
    }
}

/// Structured response to an agent prompt (plan, permission, question).
///
/// Used by `Effect::RespondToAgent` to call coop's `/api/v1/agent/respond`
/// endpoint instead of sending raw keyboard sequences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptResponse {
    /// Accept or reject the prompt (plan approval, permission grant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<bool>,
    /// Explicit option number (1-indexed) for multi-option prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<u32>,
    /// Freeform text (revision feedback, custom answer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Maximum characters to use from an agent ID when constructing directory names.
///
/// macOS limits Unix socket paths to 104 bytes (`SUN_LEN`). Using a 12-character
/// prefix instead of the full 36-character UUID keeps paths well under this limit
/// even with deep temp directories (e.g. `/var/folders/.../T/.tmpXXXXXX`).
const AGENT_DIR_PREFIX_LEN: usize = 12;

/// Construct the per-agent directory path under `state_dir`.
///
/// Uses a truncated agent ID prefix to stay within macOS's 104-byte Unix
/// socket path limit. All code that constructs paths under `{state_dir}/agents/`
/// must use this function for consistency.
pub fn agent_dir(state_dir: &Path, agent_id: &str) -> PathBuf {
    let len = agent_id.len().min(AGENT_DIR_PREFIX_LEN);
    state_dir.join("agents").join(&agent_id[..len])
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
