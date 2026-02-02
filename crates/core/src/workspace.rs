// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace identifier and lifecycle status.
//!
//! WorkspaceId is distinct from the workspace path (the workspace directory).
//! A workspace represents a managed directory that may be used by one or more
//! pipelines and has its own lifecycle independent of pipeline completion.

use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

/// Unique identifier for a workspace instance.
///
/// Workspaces are managed directories that can outlive pipelines (for debugging
/// failed runs) or be shared across related pipeline invocations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(pub String);

impl WorkspaceId {
    /// Create a new WorkspaceId from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the string value of this WorkspaceId.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for WorkspaceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for WorkspaceId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl PartialEq<str> for WorkspaceId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for WorkspaceId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl Borrow<str> for WorkspaceId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Status of a workspace in its lifecycle.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceStatus {
    /// Workspace is being created (directory creation in progress)
    #[default]
    Creating,
    /// Workspace is ready for use
    Ready,
    /// Workspace is actively being used by a pipeline or agent
    InUse {
        /// ID of the pipeline or agent using this workspace
        by: String,
    },
    /// Workspace is being cleaned up (directory removal in progress)
    Cleaning,
    /// Workspace creation or operation failed
    Failed {
        /// Reason for the failure
        reason: String,
    },
}

impl fmt::Display for WorkspaceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceStatus::Creating => write!(f, "creating"),
            WorkspaceStatus::Ready => write!(f, "ready"),
            WorkspaceStatus::InUse { by } => write!(f, "in_use({})", by),
            WorkspaceStatus::Cleaning => write!(f, "cleaning"),
            WorkspaceStatus::Failed { reason } => write!(f, "failed: {}", reason),
        }
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
