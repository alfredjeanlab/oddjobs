// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Emit commands for agent-to-daemon signaling

use anyhow::Result;
use clap::{Args, Subcommand};
use oj_core::{AgentId, AgentSignalKind, Event};
use serde::Deserialize;
use std::io::Read;

use crate::client::DaemonClient;
use crate::output::OutputFormat;

#[derive(Args)]
pub struct EmitArgs {
    #[command(subcommand)]
    pub command: EmitCommand,
}

#[derive(Subcommand)]
pub enum EmitCommand {
    /// Signal agent completion to the daemon
    #[command(name = "agent:signal")]
    AgentDone {
        /// Agent ID (required - no longer read from environment)
        #[arg(long = "agent")]
        agent_id: String,

        /// JSON payload: {"action": "complete"|"escalate", "message": "..."}
        /// If omitted, reads from stdin
        #[arg(value_name = "JSON")]
        payload: Option<String>,
    },
}

/// JSON payload structure for agent:signal command
#[derive(Debug, Deserialize)]
struct AgentDonePayload {
    #[serde(alias = "action")]
    kind: AgentSignalKind,
    #[serde(default)]
    message: Option<String>,
}

pub async fn handle(
    command: EmitCommand,
    client: &DaemonClient,
    _format: OutputFormat,
) -> Result<()> {
    match command {
        EmitCommand::AgentDone { agent_id, payload } => {
            // Read JSON from arg or stdin
            let json_str = match payload {
                Some(s) => s,
                None => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
            };

            let payload: AgentDonePayload = serde_json::from_str(&json_str).map_err(|e| {
                anyhow::anyhow!(
                    "invalid JSON payload: {}. Expected: {{\"kind\": \"complete\"|\"escalate\", \"message\": \"...\"}}",
                    e
                )
            })?;

            let event = Event::AgentSignal {
                agent_id: AgentId::new(agent_id),
                kind: payload.kind,
                message: payload.message,
            };

            client.emit_event(event).await?;
            Ok(())
        }
    }
}
