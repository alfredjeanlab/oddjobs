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
    AgentDetail, AgentEntry, AgentStatusEntry, AgentSummary, CronEntry, NamespaceStatus,
    OrphanAgent, OrphanSummary, PipelineDetail, PipelineEntry, PipelineStatusEntry,
    PipelineSummary, ProjectSummary, Query, QueueItemEntry, QueueItemSummary, QueueStatus,
    QueueSummary, Request, Response, SessionSummary, StepRecordDetail, WorkerEntry, WorkerSummary,
    WorkspaceDetail, WorkspaceEntry, WorkspaceSummary, DEFAULT_TIMEOUT, MAX_MESSAGE_SIZE,
    PROTOCOL_VERSION,
};
