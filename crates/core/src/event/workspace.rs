// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace event helpers

use super::Event;

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        Event::WorkspaceCreated { id, .. } => format!("{t} id={id}"),
        Event::WorkspaceReady { id }
        | Event::WorkspaceFailed { id, .. }
        | Event::WorkspaceDeleted { id } => format!("{t} id={id}"),
        Event::WorkspaceDrop { id, .. } => format!("{t} id={id}"),
        _ => unreachable!("not a workspace event"),
    }
}
