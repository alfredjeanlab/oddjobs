// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Mutation handlers for state-changing requests.

mod agents;
mod jobs;
mod prune_helpers;
mod workspaces;

pub(super) use self::agents::{
    handle_agent_kill, handle_agent_prune, handle_agent_resume, handle_agent_send,
};
pub(super) use self::jobs::{
    handle_job_cancel, handle_job_prune, handle_job_resume, handle_job_resume_all,
    handle_job_suspend, handle_status,
};
pub(super) use self::workspaces::{handle_workspace_drop, handle_workspace_prune};

use crate::event_bus::EventBus;
use oj_core::Event;

use super::ConnectionError;

/// Emit an event via the event bus.
///
/// Maps send errors to `ConnectionError::WalError`.
pub(super) fn emit(event_bus: &EventBus, event: Event) -> Result<(), ConnectionError> {
    event_bus.send(event).map(|_| ()).map_err(|_| ConnectionError::WalError)
}

/// Whether a step status is resumable without `--kill`.
pub(super) fn is_resumable_status(status: &oj_core::StepStatus) -> bool {
    status.is_waiting()
        || matches!(
            status,
            oj_core::StepStatus::Failed
                | oj_core::StepStatus::Pending
                | oj_core::StepStatus::Suspended
        )
}

/// Shared flags for prune operations.
pub(super) struct PruneFlags<'a> {
    pub all: bool,
    pub dry_run: bool,
    pub project: Option<&'a str>,
}
