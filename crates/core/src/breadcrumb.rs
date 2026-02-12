// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job breadcrumb data types for orphan detection.
//!
//! Breadcrumbs capture a snapshot of job state on creation and each step
//! transition. On daemon startup, they are cross-referenced with recovered
//! WAL/snapshot state to detect orphaned jobs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Breadcrumb snapshot written to disk on job creation and step transitions.
///
/// Write-only during normal operation; read-only during orphan detection at startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breadcrumb {
    pub job_id: String,
    pub project: String,
    pub kind: String,
    pub name: String,
    pub vars: HashMap<String, String>,
    pub current_step: String,
    pub step_status: String,
    pub agents: Vec<BreadcrumbAgent>,
    pub workspace_id: Option<String>,
    pub workspace_root: Option<PathBuf>,
    pub updated_at: String,
    /// Content hash of the stored runbook (for resume from orphan state).
    #[serde(default)]
    pub runbook_hash: String,
    /// Working directory where commands execute.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

/// Agent information captured in a breadcrumb.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreadcrumbAgent {
    pub agent_id: String,
    pub session_name: Option<String>,
    pub log_path: PathBuf,
}
