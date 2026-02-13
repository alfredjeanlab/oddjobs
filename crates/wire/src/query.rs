// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Query types for reading daemon state.

use std::path::PathBuf;

use oj_core::DecisionId;
use serde::{Deserialize, Serialize};

/// Query types for reading daemon state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Query {
    ListJobs,
    GetJob {
        id: String,
    },
    ListWorkspaces,
    GetWorkspace {
        id: String,
    },
    GetJobLogs {
        id: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        /// Byte offset for incremental reads (0 = start of file).
        /// When non-zero, returns only content after this offset.
        #[serde(default)]
        offset: u64,
    },
    GetAgentLogs {
        /// Job ID or agent ID (prefix match supported)
        id: String,
        /// Optional step filter (None = all steps)
        #[serde(default)]
        step: Option<String>,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        /// Byte offset for incremental reads (0 = start of file)
        #[serde(default)]
        offset: u64,
    },
    /// List all known queues in a project
    ListQueues {
        project_path: PathBuf,
        #[serde(default)]
        project: String,
    },
    /// List items in a persisted queue
    ListQueueItems {
        queue: String,
        #[serde(default)]
        project: String,
        #[serde(default)]
        project_path: Option<PathBuf>,
    },
    /// Get detailed info for a single agent by ID (or prefix)
    GetAgent {
        agent_id: String,
    },
    /// List agents across all jobs
    ListAgents {
        /// Filter by job ID prefix
        #[serde(default)]
        job_id: Option<String>,
        /// Filter by status (e.g. "running", "completed", "failed", "waiting")
        #[serde(default)]
        status: Option<String>,
    },
    /// Get worker activity logs
    GetWorkerLogs {
        name: String,
        #[serde(default)]
        project: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        #[serde(default)]
        project_path: Option<PathBuf>,
        /// Byte offset for incremental reads (0 = start of file)
        #[serde(default)]
        offset: u64,
    },
    /// List all workers and their status
    ListWorkers,
    /// List all crons and their status
    ListCrons,
    /// Get cron activity logs
    GetCronLogs {
        /// Cron name
        name: String,
        #[serde(default)]
        project: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        #[serde(default)]
        project_path: Option<PathBuf>,
        /// Byte offset for incremental reads (0 = start of file)
        #[serde(default)]
        offset: u64,
    },
    /// Get a cross-project status overview
    StatusOverview,
    /// List all projects with active work
    ListProjects,
    /// List orphaned jobs detected from breadcrumbs at startup
    ListOrphans,
    /// Dismiss an orphaned job by ID
    DismissOrphan {
        id: String,
    },
    /// Get queue activity logs
    GetQueueLogs {
        queue: String,
        #[serde(default)]
        project: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        /// Byte offset for incremental reads (0 = start of file)
        #[serde(default)]
        offset: u64,
    },
    /// List pending decisions (optionally filtered by project)
    ListDecisions {
        #[serde(default)]
        project: String,
    },
    /// Get a single decision by ID (prefix match supported)
    GetDecision {
        id: DecisionId,
    },
}
