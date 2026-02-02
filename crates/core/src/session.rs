// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Session identifier type for tracking agent sessions.
//!
//! SessionId identifies an agent's underlying session (e.g., a tmux session).
//! This is distinct from AgentId which identifies the logical agent instance.

use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

/// Unique identifier for an agent session.
///
/// Sessions represent the underlying execution environment for agents,
/// such as tmux sessions. Multiple agent invocations may share a session,
/// or each agent may have its own dedicated session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    /// Create a new SessionId from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the string value of this SessionId.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl PartialEq<str> for SessionId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for SessionId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl Borrow<str> for SessionId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
