// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Query types for reading daemon state.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Query types for reading daemon state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Query {
    ListJobs,
    GetJob {
        id: String,
    },
    ListSessions,
    /// Get a single session by ID (exact or prefix match)
    GetSession {
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
    },
    GetAgentLogs {
        /// Job ID (not agent_id anymore)
        id: String,
        /// Optional step filter (None = all steps)
        #[serde(default)]
        step: Option<String>,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
    },
    /// Query if an agent has signaled completion (for stop hook)
    GetAgentSignal {
        agent_id: String,
    },
    /// List all known queues in a project
    ListQueues {
        project_root: PathBuf,
        #[serde(default)]
        namespace: String,
    },
    /// List items in a persisted queue
    ListQueueItems {
        queue_name: String,
        #[serde(default)]
        namespace: String,
        #[serde(default)]
        project_root: Option<PathBuf>,
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
        namespace: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        #[serde(default)]
        project_root: Option<PathBuf>,
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
        namespace: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
        #[serde(default)]
        project_root: Option<PathBuf>,
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
        queue_name: String,
        #[serde(default)]
        namespace: String,
        /// Number of most recent lines to return (0 = all)
        lines: usize,
    },
    /// List pending decisions (optionally filtered by namespace)
    ListDecisions {
        #[serde(default)]
        namespace: String,
    },
    /// Get a single decision by ID (prefix match supported)
    GetDecision {
        id: String,
    },
}
