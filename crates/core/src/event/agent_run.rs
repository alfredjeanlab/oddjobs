// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent run event helpers

use super::{ns_fragment, Event};

pub(super) fn log_summary(event: &Event, t: &str) -> String {
    match event {
        Event::AgentRunCreated {
            id,
            agent_name,
            namespace,
            ..
        } => {
            format!("{t} id={id}{} agent={agent_name}", ns_fragment(namespace))
        }
        Event::AgentRunStarted { id, agent_id } => {
            format!("{t} id={id} agent_id={agent_id}")
        }
        Event::AgentRunStatusChanged { id, status, reason } => {
            if let Some(reason) = reason {
                format!("{t} id={id} status={status} reason={reason}")
            } else {
                format!("{t} id={id} status={status}")
            }
        }
        Event::AgentRunResume { id, message, kill } => {
            if *kill {
                format!("{t} id={id} kill=true")
            } else if message.is_some() {
                format!("{t} id={id} msg=true")
            } else {
                format!("{t} id={id}")
            }
        }
        Event::AgentRunDeleted { id } => format!("{t} id={id}"),
        _ => unreachable!("not an agent_run event"),
    }
}
