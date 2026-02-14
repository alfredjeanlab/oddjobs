// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace identifier and lifecycle status.
//!
//! WorkspaceId is distinct from the workspace path (the workspace directory).
//! A workspace represents a managed directory that may be used by one or more
//! jobs and has its own lifecycle independent of job completion.

use serde::{Deserialize, Serialize};
use std::fmt;

crate::define_id! {
    /// Unique identifier for a workspace instance.
    ///
    /// Workspaces are managed directories that can outlive jobs (for debugging
    /// failed runs) or be shared across related job invocations.
    pub struct WorkspaceId("wks-");
}

/// Status of a workspace in its lifecycle.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceStatus {
    /// Workspace is being created (directory creation in progress)
    #[default]
    Creating,
    /// Workspace is ready for use
    Ready,
    /// Workspace is actively being used by a job or agent
    InUse {
        /// ID of the job or agent using this workspace
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
