// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Mutation handlers for state-changing requests.

mod agents;
mod jobs;
mod prune_helpers;
mod resources;
mod sessions;
mod workspaces;

pub(super) use self::agents::{
    handle_agent_kill, handle_agent_prune, handle_agent_resume, handle_agent_send,
};
pub(super) use self::jobs::{
    handle_job_cancel, handle_job_prune, handle_job_resume, handle_job_resume_all, handle_status,
};
pub(super) use self::resources::{handle_cron_prune, handle_worker_prune};
pub(super) use self::sessions::{handle_session_kill, handle_session_prune, handle_session_send};
pub(super) use self::workspaces::{handle_workspace_drop, handle_workspace_prune};

use crate::event_bus::EventBus;
use oj_core::Event;

use super::ConnectionError;

/// Emit an event via the event bus.
///
/// Maps send errors to `ConnectionError::WalError`.
pub(super) fn emit(event_bus: &EventBus, event: Event) -> Result<(), ConnectionError> {
    event_bus
        .send(event)
        .map(|_| ())
        .map_err(|_| ConnectionError::WalError)
}

/// Shared flags for prune operations.
pub(super) struct PruneFlags<'a> {
    pub all: bool,
    pub dry_run: bool,
    pub namespace: Option<&'a str>,
}

#[cfg(test)]
#[path = "mutations_tests.rs"]
mod tests;
