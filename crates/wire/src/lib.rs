// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! IPC Protocol for daemon communication.
//!
//! Wire format: 4-byte length prefix (big-endian) + JSON payload

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

mod query;
mod request;
mod response;
mod status;
mod types;
mod wire;

pub use query::Query;
pub use request::Request;
pub use response::Response;
pub use status::{
    parse_step_status_kind, AgentEntry, AgentStatusEntry, CronEntry, CronSummary, JobEntry,
    JobStatusEntry, MetricsHealthSummary, OrphanAgent, OrphanSummary, ProjectStatus,
    ProjectSummary, QueueItemEntry, QueueStatus, WorkerEntry,
};
pub use types::{
    AgentDetail, AgentSummary, DecisionDetail, DecisionSummary, JobDetail, JobSummary,
    QueueItemSummary, QueueSummary, StepRecordDetail, WorkerSummary, WorkspaceDetail,
    WorkspaceEntry, WorkspaceSummary,
};
pub use types::{DecisionOptionDetail, QuestionGroupDetail};
pub use wire::{decode, encode, read_message, write_message, ProtocolError};
pub use wire::{read_request, write_response};

#[cfg(test)]
mod property_tests;