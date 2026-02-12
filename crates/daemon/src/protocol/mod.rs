// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! IPC Protocol for daemon communication.
//!
//! Wire format: 4-byte length prefix (big-endian) + JSON payload

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
    AgentEntry, AgentStatusEntry, CronEntry, CronSummary, JobEntry, JobStatusEntry,
    MetricsHealthSummary, OrphanSummary, ProjectStatus, ProjectSummary, QueueItemEntry,
    QueueStatus, WorkerEntry,
};
pub use types::{
    AgentDetail, AgentSummary, DecisionDetail, DecisionSummary, JobDetail, JobSummary,
    QueueItemSummary, QueueSummary, StepRecordDetail, WorkerSummary, WorkspaceDetail,
    WorkspaceEntry, WorkspaceSummary,
};

// Exported for `crates/cli`
#[allow(unused_imports)]
pub use types::{DecisionOptionDetail, QuestionGroupDetail};

// Exported for `crates/cli`
#[allow(unused_imports)]
pub use wire::{decode, encode, read_message, write_message, ProtocolError};
pub use wire::{read_request, write_response};

#[cfg(test)]
mod property_tests;
