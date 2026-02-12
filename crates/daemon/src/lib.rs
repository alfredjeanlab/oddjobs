// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Odd Jobs Daemon library
//!
//! This module exposes the IPC protocol types for use by CLI clients.

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

pub mod protocol;

pub use protocol::{
    AgentDetail, AgentEntry, AgentStatusEntry, AgentSummary, CronEntry, CronSummary, JobDetail,
    JobEntry, JobStatusEntry, JobSummary, MetricsHealthSummary, OrphanSummary, ProjectStatus,
    ProjectSummary, Query, QueueItemEntry, QueueItemSummary, QueueStatus, QueueSummary, Request,
    Response, StepRecordDetail, WorkerEntry, WorkerSummary, WorkspaceDetail, WorkspaceEntry,
    WorkspaceSummary,
};
